"""Bounded HTTPS subscription reads; candidates are never health-checked here."""
from __future__ import annotations

from datetime import datetime, timezone
import ipaddress
import json
import queue
import re
import socket
import threading
import time
from urllib.parse import urlsplit, urlunsplit

import idna
import requests
from requests.adapters import HTTPAdapter

from proxy_import import MAX_IMPORTED_PROXIES, make_record, parse_proxy_text

MAX_RESPONSE_BYTES = 2 * 1024 * 1024
MAX_URL_LENGTH = 8192
DNS_TIMEOUT_SECONDS = 5
FETCH_TIMEOUT_SECONDS = 20
_WORK_SLOTS = threading.BoundedSemaphore(2)


class SubscriptionError(ValueError):
    """Only fixed, credential-free messages may cross this boundary."""


class SubscriptionCancelled(SubscriptionError):
    pass


def _public_ip(value):
    try:
        address = ipaddress.ip_address(value)
    except ValueError:
        raise SubscriptionError('订阅地址解析结果无效') from None
    if (not address.is_global or address.is_multicast or address.is_reserved
            or address.is_unspecified or address.is_loopback or address.is_link_local
            or '%' in value):
        raise SubscriptionError('订阅地址必须指向公网，不能访问本机或内网')
    return address.compressed


def _parse_url(url):
    if (not isinstance(url, str) or not url or len(url) > MAX_URL_LENGTH
            or url != url.strip() or '\\' in url
            or any(ord(char) <= 32 or ord(char) == 127 for char in url)):
        raise SubscriptionError('订阅地址格式无效，请填写完整 HTTPS 地址')
    try:
        parts = urlsplit(url)
        if (parts.scheme.lower() != 'https' or not parts.hostname or parts.fragment
                or parts.username is not None or parts.password is not None):
            raise ValueError
        port = parts.port or 443
        if not 1 <= port <= 65535 or parts.port == 0:
            raise ValueError
        host = parts.hostname.rstrip('.')
        if not host or '%' in host:
            raise ValueError
        try:
            literal = ipaddress.ip_address(host)
        except ValueError:
            host = idna.encode(host, uts46=True, std3_rules=True).decode('ascii').lower()
            if ('.' not in host or host.endswith(('.localhost', '.local', '.internal', '.invalid', '.test', '.onion'))
                    or host == 'localhost'):
                raise SubscriptionError('订阅地址必须使用公网主机名')
        else:
            host = _public_ip(str(literal))
        authority = f'[{host}]' if ':' in host else host
        if port != 443:
            authority += f':{port}'
        canonical = urlunsplit(('https', authority, parts.path or '/', parts.query, ''))
        return canonical, {'host': host, 'port': port, 'label': authority, 'has_query': bool(parts.query)}
    except SubscriptionError:
        raise
    except (ValueError, UnicodeError):
        raise SubscriptionError('订阅地址须为 HTTPS，不能包含账号密码或片段') from None


def subscription_summary(url):
    """Pure local validation and redaction; does not resolve or contact a host."""
    return _parse_url(url)[1]


def _check_cancelled(cancel):
    if cancel.is_set():
        raise SubscriptionCancelled('订阅获取已取消')


def _run_bounded(operation, stop_event, timeout):
    """Bound caller time even if OS DNS or a peer stalls; at most two workers.

    These are one-shot daemon workers, never polling/refresh schedulers. A
    cancelled worker exits at its next I/O boundary; its slot remains occupied
    until then so repeated cancellation cannot create unbounded background I/O.
    """
    if stop_event is not None and stop_event.is_set():
        raise SubscriptionCancelled('订阅获取已取消')
    if not _WORK_SLOTS.acquire(blocking=False):
        raise SubscriptionError('订阅获取仍在处理，请稍后重试')
    cancelled, result = threading.Event(), queue.Queue(maxsize=1)
    def work():
        try:
            result.put((True, operation(cancelled)))
        except SubscriptionError as error:
            result.put((False, error))
        except Exception:
            result.put((False, SubscriptionError('订阅获取失败，请检查地址和网络')))
        finally:
            _WORK_SLOTS.release()
    worker = threading.Thread(target=work, name='subscription-fetch', daemon=True)
    try:
        worker.start()
    except Exception:
        _WORK_SLOTS.release()
        raise SubscriptionError('暂时无法启动订阅获取，请稍后重试') from None
    deadline = time.monotonic() + timeout
    while True:
        if stop_event is not None and stop_event.is_set():
            cancelled.set()
            raise SubscriptionCancelled('订阅获取已取消')
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            cancelled.set()
            raise SubscriptionError('订阅获取超时，请稍后重试')
        try:
            succeeded, value = result.get(timeout=min(0.05, remaining))
        except queue.Empty:
            continue
        if stop_event is not None and stop_event.is_set():
            cancelled.set()
            raise SubscriptionCancelled('订阅获取已取消')
        if not succeeded:
            raise value from None
        return value


def _resolve_public(host, port, cancel):
    _check_cancelled(cancel)
    try:
        literal = ipaddress.ip_address(host)
    except ValueError:
        try:
            answers = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM, proto=socket.IPPROTO_TCP)
        except OSError:
            raise SubscriptionError('无法解析订阅主机，请检查地址和网络') from None
        _check_cancelled(cancel)
        addresses = []
        for family, _, _, _, sockaddr in answers:
            if family not in (socket.AF_INET, socket.AF_INET6):
                raise SubscriptionError('订阅地址解析结果无效')
            address = _public_ip(sockaddr[0])
            if address not in addresses:
                addresses.append(address)
        if not addresses:
            raise SubscriptionError('订阅主机未返回公网地址')
        # Prefer IPv4 where both are available; no alternate DNS resolution.
        return sorted(addresses, key=lambda value: ':' in value)
    else:
        return [_public_ip(str(literal))]


def validate_subscription_url(url):
    """Normalize a URL and check every current DNS answer is public."""
    canonical, summary = _parse_url(url)
    _run_bounded(lambda cancel: _resolve_public(summary['host'], summary['port'], cancel), None, DNS_TIMEOUT_SECONDS)
    return canonical


class _PinnedHTTPSAdapter(HTTPAdapter):
    """Connect to a validated numeric IP while verifying the original TLS name."""
    def __init__(self, host, port, address):
        self._host, self._port, self._address = host, port, address
        super().__init__(max_retries=0, pool_connections=1, pool_maxsize=1)

    def get_connection_with_tls_context(self, request, verify, proxies=None, cert=None):
        _, summary = _parse_url(request.url)
        if (summary['host'], summary['port']) != (self._host, self._port) or verify is not True or proxies:
            raise SubscriptionError('订阅连接安全校验失败')
        host_params, pool_kwargs = self.build_connection_pool_key_attributes(request, verify, cert)
        host_params['host'] = self._address
        pool_kwargs.update(assert_hostname=self._host, server_hostname=self._host)
        return self.poolmanager.connection_from_host(**host_params, pool_kwargs=pool_kwargs)


def _parse_candidates(text):
    records, rejected, duplicates, seen = [], 0, 0, set()
    text = text.lstrip('\ufeff').strip()
    if not text:
        raise SubscriptionError('订阅未返回候选代理')
    try:
        is_json = text.startswith(('{', '[')) and not re.match(r'^\[[0-9a-fA-F:]+\]:', text)
        geonode = False
        if is_json:
            payload = json.loads(text)
            if isinstance(payload, dict):
                if not isinstance(payload.get('data'), list):
                    raise SubscriptionError('订阅 JSON 未包含代理数组')
                rows, geonode = payload['data'], True
            elif isinstance(payload, list):
                rows = payload
            else:
                raise SubscriptionError('订阅 JSON 格式不受支持')
        else:
            rows = [line.strip() for line in text.splitlines() if line.strip() and not line.lstrip().startswith('#')]
        if len(rows) > MAX_IMPORTED_PROXIES:
            raise SubscriptionError('订阅候选超过 5000 条，请缩小订阅范围')
        for row in rows:
            try:
                if geonode:
                    if not isinstance(row, dict) or not isinstance(row.get('protocols'), list):
                        raise ValueError
                    protocols = {value.lower() for value in row['protocols'] if isinstance(value, str)}
                    protocol = 'socks5h' if protocols & {'socks5', 'socks5h'} else 'http' if protocols & {'http', 'https'} else None
                    if protocol is None:
                        raise ValueError
                    record = make_record({'ip': row.get('ip'), 'port': row.get('port'), 'protocol': protocol,
                                          'country_code': row.get('country') if isinstance(row.get('country'), str) else ''})
                elif is_json:
                    record = make_record(row)
                else:
                    parsed, result = parse_proxy_text(row)
                    if len(parsed) != 1 or result['invalid']:
                        raise ValueError
                    record = parsed[0]
                if record['id'] in seen:
                    duplicates += 1
                else:
                    seen.add(record['id'])
                    records.append(record)
            except (ValueError, TypeError):
                rejected += 1
        if rows and not records:
            raise SubscriptionError('订阅没有支持的有效代理，请检查内容格式和代理协议')
        return records, rejected, duplicates
    except SubscriptionError:
        raise
    except (ValueError, TypeError, RecursionError):
        raise SubscriptionError('订阅内容格式无效，请使用代理 JSON 或逐行代理地址') from None


def _fetch(canonical, summary, cancel):
    addresses = _resolve_public(summary['host'], summary['port'], cancel)
    _check_cancelled(cancel)
    with requests.Session() as session:
        session.trust_env = False
        session.mount('https://', _PinnedHTTPSAdapter(summary['host'], summary['port'], addresses[0]))
        headers = {'Host': summary['label'], 'Accept': 'application/json, text/plain',
                   'Accept-Encoding': 'identity', 'User-Agent': 'QingkanLive-Subscription/1'}
        with session.get(canonical, headers=headers, timeout=(5, 5), verify=True, proxies={},
                         allow_redirects=False, stream=True) as response:
            if response.status_code != 200:
                raise SubscriptionError(f'订阅服务返回 HTTP {int(response.status_code)}，未导入候选')
            encoding = response.headers.get('Content-Encoding', 'identity').lower().strip()
            if encoding not in ('', 'identity'):
                raise SubscriptionError('订阅响应压缩格式不受支持，请使用未压缩订阅')
            length = response.headers.get('Content-Length')
            if length is not None and (not length.isascii() or not length.isdigit() or int(length) > MAX_RESPONSE_BYTES):
                raise SubscriptionError('订阅响应过大或长度无效，最大支持 2 MiB')
            data = bytearray()
            for chunk in response.iter_content(16384):
                _check_cancelled(cancel)
                if len(data) + len(chunk) > MAX_RESPONSE_BYTES:
                    raise SubscriptionError('订阅响应过大，最大支持 2 MiB')
                data.extend(chunk)
            _check_cancelled(cancel)
            try:
                text = data.decode('utf-8-sig')
            except UnicodeError:
                raise SubscriptionError('订阅内容须为 UTF-8 文本') from None
    records, rejected, duplicates = _parse_candidates(text)
    _check_cancelled(cancel)
    return {'records': records, 'count': len(records), 'rejected': rejected, 'duplicates': duplicates,
            'fetched_at': datetime.now(timezone.utc).isoformat(timespec='seconds'), 'source': summary}


def fetch_subscription(url, stop_event=None):
    """Fetch candidates only. Returned records may contain write-only credentials."""
    canonical, summary = _parse_url(url)
    return _run_bounded(lambda cancel: _fetch(canonical, summary, cancel), stop_event, FETCH_TIMEOUT_SECONDS)
