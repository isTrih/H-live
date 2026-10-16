"""Parse proxy imports without echoing credentials or trusting vendor status."""
from __future__ import annotations

import csv
from datetime import datetime, timedelta, timezone
import hashlib
import json
import re
from urllib.parse import unquote, urlsplit

from proxy_pool import normalize_endpoint

MAX_IMPORTED_PROXIES = 5000
MAX_IMPORT_TEXT = 800_000


class ProxyImportError(ValueError):
    """Contains deliberately generic text, never an input line."""


def normalize_protocol(value):
    if not isinstance(value, str) or value.lower() not in {'http', 'socks5', 'socks5h'}:
        raise ProxyImportError('代理协议仅支持 HTTP CONNECT 或 SOCKS5')
    return 'http' if value.lower() == 'http' else 'socks5h'


def _credential(value):
    if not isinstance(value, str) or len(value) > 512 or any(ord(c) < 32 or ord(c) == 127 for c in value):
        raise ProxyImportError('代理账号或密码格式无效')
    return value


def _expiry(value):
    if value in (None, '', '-', '永久', '不限'):
        return None
    if not isinstance(value, str) or len(value) > 40:
        raise ProxyImportError('到期时间格式无效')
    try:
        parsed = datetime.fromisoformat(value.strip().replace('Z', '+00:00'))
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=timezone(timedelta(hours=8)))
        return parsed.isoformat(timespec='seconds')
    except ValueError:
        raise ProxyImportError('到期时间须为有效日期时间或留空') from None


def make_record(value, default_protocol='http'):
    if not isinstance(value, dict):
        raise ProxyImportError('每条代理须为对象')
    protocol = normalize_protocol(value.get('protocol') or default_protocol)
    endpoint = value.get('endpoint')
    if endpoint is None:
        ip, port = value.get('ip'), value.get('port')
        if not isinstance(ip, str) or isinstance(port, bool) or not isinstance(port, (str, int)):
            raise ProxyImportError('代理须包含 IP 和数字端口')
        ip = ip.strip()
        if ':' in ip and not ip.startswith('['):
            ip = f'[{ip}]'
        endpoint = f'{ip}:{port}'
    try:
        endpoint = normalize_endpoint(endpoint)
    except (ValueError, TypeError):
        raise ProxyImportError('代理 IP 或端口无效；仅接受公网 IPv4 / IPv6 地址') from None
    username = _credential(value.get('username', ''))
    password = _credential(value.get('password', ''))
    if bool(username) != bool(password):
        raise ProxyImportError('代理账号和密码须同时填写或同时留空')
    if protocol == 'http' and (not username.isascii() or not password.isascii() or ':' in username):
        raise ProxyImportError('HTTP 代理账号密码须为 ASCII 字符')
    if protocol == 'socks5h' and any(len(item.encode('utf-8')) > 255 for item in (username, password)):
        raise ProxyImportError('SOCKS5 账号密码各不得超过 255 个 UTF-8 字节')
    record = {'id': hashlib.sha256(endpoint.encode()).hexdigest()[:24],
              'endpoint': endpoint, 'protocol': protocol, 'username': username, 'password': password,
              'expires_at': _expiry(value.get('expires_at'))}
    for key in ('city', 'country_code'):
        item = value.get(key, '')
        if item is None:
            item = ''
        if not isinstance(item, str) or len(item) > 100 or any(ord(c) < 32 for c in item):
            raise ProxyImportError('代理地区信息格式无效')
        record[key] = item
    return record


def validate_records(values):
    if not isinstance(values, list) or len(values) > MAX_IMPORTED_PROXIES:
        raise ProxyImportError('最多保存 5000 条导入代理')
    records, seen = [], set()
    for value in values:
        record = make_record(value)
        if record['id'] in seen:
            raise ProxyImportError('保存的代理列表含重复地址')
        records.append(record)
        seen.add(record['id'])
    return records


def public_record(record):
    return {key: record.get(key) for key in ('id', 'endpoint', 'protocol', 'city', 'country_code', 'expires_at')} | {
        'auth_present': bool(record.get('username')), 'status': 'unchecked'}


_HEADERS = {
    'ip': 'ip', 'ip地址': 'ip', '地址': 'ip', '服务器ip': 'ip',
    '端口': 'port', 'port': 'port', '账号': 'username', '帐号': 'username',
    '用户名': 'username', 'username': 'username', 'user': 'username',
    '密码': 'password', 'password': 'password', 'pass': 'password',
    '城市': 'city', 'city': 'city', '国家': 'country_code', 'country_code': 'country_code',
    '协议': 'protocol', 'protocol': 'protocol', '到期': 'expires_at', '到期时间': 'expires_at',
    '过期时间': 'expires_at', 'expires_at': 'expires_at',
    '更新时间': '_updated', '更新': '_updated', 'updated_at': '_updated', '状态': '_status', 'status': '_status',
}


def _cells(line):
    if '\t' in line:
        return [part.strip() for part in line.split('\t')]
    if '|' in line:
        return [part.strip() for part in line.strip('|').split('|')]
    if ',' in line:
        return [part.strip() for part in next(csv.reader([line]))]
    return line.split()


def _from_line(line, header, default_protocol):
    if re.match(r'^[a-zA-Z][a-zA-Z0-9+.-]*://', line):
        try:
            parsed = urlsplit(line)
            protocol = normalize_protocol(parsed.scheme)
            if parsed.path not in ('', '/') or parsed.query or parsed.fragment or parsed.hostname is None or parsed.port is None:
                raise ValueError
            host = parsed.hostname
            if ':' in host:
                host = f'[{host}]'
            for component in (parsed.username or '', parsed.password or ''):
                if re.search(r'%(?![0-9a-fA-F]{2})', component):
                    raise ValueError
            return make_record({'endpoint': f'{host}:{parsed.port}', 'protocol': protocol,
                                'username': unquote(parsed.username or '', errors='strict'),
                                'password': unquote(parsed.password or '', errors='strict')})
        except (ValueError, UnicodeError):
            raise ProxyImportError('代理 URL 格式无效或协议不受支持') from None
    if header:
        cells = _cells(line)
        if '\t' not in line and '|' not in line and ',' not in line and len(cells) > len(header):
            combined, position = [], 0
            for key in header:
                if position >= len(cells):
                    break
                cell = cells[position]
                position += 1
                if (key in ('_updated', 'expires_at') and re.fullmatch(r'\d{4}-\d{2}-\d{2}', cell)
                        and position < len(cells) and re.fullmatch(r'\d{2}:\d{2}:\d{2}', cells[position])):
                    cell += ' ' + cells[position]
                    position += 1
                combined.append(cell)
            if position == len(cells):
                cells = combined
        if len(cells) != len(header):
            raise ProxyImportError('表格列数与标题不一致')
        fields = {key: cell for key, cell in zip(header, cells) if key and not key.startswith('_')}
        return make_record(fields, default_protocol)
    match = re.fullmatch(r'(\[[^\]]+\]|[^:\s]+):([0-9]+)(?::([^:\s]+):(.+))?', line)
    if match:
        return make_record({'endpoint': f'{match[1]}:{match[2]}', 'username': match[3] or '',
                            'password': match[4] or ''}, default_protocol)
    cells = _cells(line)
    if len(cells) == 4:
        return make_record(dict(zip(('ip', 'port', 'username', 'password'), cells)), default_protocol)
    # Headerless vendor tables have one explicitly recognized shape. Dates
    # must occupy their own documented positions; never infer auth from city.
    if len(cells) == 10 and re.fullmatch(r'\d{2}:\d{2}:\d{2}', cells[6]) and re.fullmatch(r'\d{2}:\d{2}:\d{2}', cells[8]):
        cells = cells[:5] + [cells[5] + ' ' + cells[6], cells[7] + ' ' + cells[8], cells[9]]
    if len(cells) == 8:
        if not re.fullmatch(r'\d{4}-\d{2}-\d{2}(?:[ T].*)?', cells[5]):
            raise ProxyImportError('无标题表格须按 IP、城市、端口、账号、密码、更新时间、到期时间、状态排列')
        _expiry(cells[5])
        return make_record(dict(zip(('ip', 'city', 'port', 'username', 'password', '_updated', 'expires_at', '_status'), cells)), default_protocol)
    raise ProxyImportError('无法识别此行；请提供带标题表格、IP:端口:账号:密码 或代理 URL')


def parse_proxy_text(text, default_protocol='http', existing=()):
    default_protocol = normalize_protocol(default_protocol)
    if not isinstance(text, str) or len(text) > MAX_IMPORT_TEXT:
        raise ProxyImportError('导入文本须不超过 800,000 字符')
    text = text.strip().lstrip('\ufeff')
    records, errors, duplicates, total = [], [], 0, 0
    existing_by_id = {item['id']: item for item in existing}
    seen = set()
    added, updated = 0, 0
    header = None
    if text.startswith('{') or (text.startswith('[') and not re.match(r'^\[[0-9a-fA-F:]+\]:', text)):
        try:
            rows = json.loads(text)
        except ValueError:
            raise ProxyImportError('JSON 格式无效；原始内容不会写入日志') from None
        if not isinstance(rows, list):
            raise ProxyImportError('JSON 顶层须为代理对象数组')
        source = enumerate(rows, 1)
        is_json = True
    else:
        source = enumerate(text.splitlines(), 1)
        is_json = False
    for number, value in source:
        if not is_json:
            value = value.strip()
            if not value or value.startswith('#') or re.fullmatch(r'[\s|:\-]+', value):
                continue
            cells = _cells(value)
            maybe_header = [_HEADERS.get(cell.strip().lower()) for cell in cells]
            if 'ip' in maybe_header and 'port' in maybe_header:
                if ('username' in maybe_header) != ('password' in maybe_header):
                    raise ProxyImportError('标题须同时包含账号、密码列，或都不包含')
                known = [key for key in maybe_header if key]
                if len(set(known)) != len(known):
                    raise ProxyImportError('表格标题含重复列名，请明确 IP、端口、账号和密码列')
                header = maybe_header
                continue
        total += 1
        if total > MAX_IMPORTED_PROXIES:
            raise ProxyImportError('每次最多导入 5000 条代理')
        try:
            record = make_record(value, default_protocol) if is_json else _from_line(value, header, default_protocol)
            if record['id'] in seen:
                duplicates += 1
                continue
            seen.add(record['id'])
            previous = existing_by_id.get(record['id'])
            if previous == record:
                duplicates += 1
                continue
            if previous is None:
                added += 1
            else:
                updated += 1
            records.append(record)
        except (ProxyImportError, ValueError, TypeError):
            errors.append({'line': number, 'message': '此行的代理地址、协议、账号列或日期格式无效'})
    return records, {'total': total, 'valid': len(records), 'added': added, 'updated': updated,
                     'duplicates': duplicates, 'invalid': len(errors),
                     'items': [public_record(record) for record in records], 'errors': errors}
