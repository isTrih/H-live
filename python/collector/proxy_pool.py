"""HTTPS-source proxy leases using verified HTTPS over HTTP CONNECT."""

from __future__ import annotations

from collections import OrderedDict, deque
from concurrent.futures import ThreadPoolExecutor, wait
from dataclasses import dataclass, field, replace
from datetime import datetime, timezone
import ipaddress
import json
import math
import re
import threading
import time
from urllib.parse import quote

import requests
from socks import SOCKS5AuthError
from route_control import RouteControl

API_URL = "https://proxy.scdn.io/api/get_proxy.php"
SOURCE_PROTOCOL = "https"
HEALTH_URL = "https://live.douyin.com/"
AUTH_FAILURE_MESSAGE = "代理认证失败，请检查账号、密码或供应商访问权限"


def _is_proxy_auth_failure(error):
    """Inspect typed transport causes without returning their sensitive text."""
    pending, visited = [error], set()
    while pending and len(visited) < 32:
        current = pending.pop()
        if not isinstance(current, BaseException) or id(current) in visited:
            continue
        visited.add(id(current))
        if isinstance(current, SOCKS5AuthError):
            return True
        if isinstance(current, OSError) and re.match(r'^Tunnel connection failed:\s*407(?:\s|$)', str(current), re.IGNORECASE):
            return True
        pending.extend(value for value in current.args if isinstance(value, BaseException))
        pending.extend(getattr(current, name, None) for name in
                       ('__cause__', '__context__', 'reason', 'original_error', 'socket_err'))
    return False


is_proxy_auth_failure = _is_proxy_auth_failure


class ProxyPoolError(RuntimeError):
    """A credential-free error suitable for application diagnostics."""


class ProxyFailureRegistry:
    """Share failed endpoints between pools for one application lifetime."""

    def __init__(self):
        self._lock = threading.Lock()
        self._endpoints = set()

    def add(self, endpoint):
        with self._lock:
            self._endpoints.add(endpoint)

    def contains(self, endpoint):
        with self._lock:
            return endpoint in self._endpoints

    def count(self):
        with self._lock:
            return len(self._endpoints)

    def discard(self, endpoint):
        """Allow an explicitly replaced imported route to be checked again."""
        with self._lock:
            self._endpoints.discard(endpoint)


def normalize_endpoint(value):
    if not isinstance(value, str) or not value or value != value.strip():
        raise ValueError("代理必须为公网 IP:端口")
    if value.startswith("["):
        closing = value.find("]:")
        if closing < 0:
            raise ValueError("代理地址格式无效")
        host, port = value[1:closing], value[closing + 2:]
    else:
        host, separator, port = value.rpartition(":")
        if not separator or ":" in host:
            raise ValueError("代理地址格式无效")
    if '%' in host or not port.isascii() or not port.isdigit() or not 1 <= int(port) <= 65535:
        raise ValueError("代理端口无效")
    address = ipaddress.ip_address(host)
    if not address.is_global or address.is_multicast or address.is_reserved or address.is_unspecified:
        raise ValueError("代理地址必须为公网 IP")
    host = f"[{address.compressed}]" if address.version == 6 else address.compressed
    return f"{host}:{int(port)}"


def fetch_candidates(count=20):
    """Fetch only the fixed source; ignore environment proxies and netrc."""
    with requests.Session() as session:
        session.trust_env = False
        # Omit the optional country filter: the source currently treats an
        # explicit 'all' as a country code and returns an empty list.
        with session.get(API_URL, params={"protocol": SOURCE_PROTOCOL, "count": min(20, max(1, int(count)))}, timeout=(3, 5), verify=True,
                         allow_redirects=False, stream=True) as response:
            if response.status_code != 200:
                raise ProxyPoolError(f"代理源返回 HTTP {response.status_code}")
            data = bytearray()
            for chunk in response.iter_content(8192):
                data.extend(chunk)
                if len(data) > 262144:
                    raise ProxyPoolError("代理源响应过大")
            try:
                payload = json.loads(data)
            except (ValueError, UnicodeError) as exc:
                raise ProxyPoolError("代理源响应格式无效") from exc
    if not isinstance(payload, dict) or payload.get("code") != 200:
        raise ProxyPoolError("代理源暂时无法提供候选")
    data = payload.get("data")
    if not isinstance(data, dict) or not isinstance(data.get("proxies"), list):
        raise ProxyPoolError("代理源未返回候选列表")
    return data["proxies"][:20]


def validate_proxy(endpoint, *, proxy_url=None):
    """Homepage preflight only: TLS-verified CONNECT, no user credentials.

    Success does not establish room API or WebSocket availability. Those are
    checked by the monitor through the same lease when a task starts.
    """
    endpoint = normalize_endpoint(endpoint)
    proxy = proxy_url or f"http://{endpoint}"
    started = time.monotonic()
    with requests.Session() as session:
        session.trust_env = False
        with session.get(HEALTH_URL, proxies={"http": proxy, "https": proxy},
                         headers={"User-Agent": "Mozilla/5.0"}, timeout=(3, 5), verify=True,
                         allow_redirects=False, stream=True) as response:
            code = int(response.status_code)
    return {"ok": code == 200, "status_code": code,
            "latency_ms": round((time.monotonic() - started) * 1000, 1),
            "error": "" if code == 200 else f"目标服务返回 HTTP {code}"}


def _iso():
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


@dataclass(frozen=True)
class ProxyLease:
    endpoint: str
    proxy_url: str = field(repr=False)
    status_code: int
    latency_ms: float
    verified_at: str
    acquired_at: str = ""
    rid: str = ""
    verified_monotonic: float = 0
    protocol: str = "http"
    auth_present: bool = False
    kind: str = "proxy"
    route_id: str = ""
    control: RouteControl = field(default_factory=RouteControl, repr=False, compare=False)
    generation: int = field(default=0, repr=False)


class ProxyPool:
    """Room leases over validated exits; initialization performs no network I/O."""

    def __init__(self, fetcher=None, validator=None, max_checks=12, ttl_seconds=300,
                 request_interval_seconds=1, max_candidates=600, failure_registry=None,
                 refresh_max_requests=10, refresh_timeout_seconds=90, imported_candidates=None,
                 shared=False, include_direct=False, warmup_seconds=8,
                 target_cooldown_seconds=60, transport_cooldown_seconds=30,
                 transport_window_seconds=30, route_request_interval_seconds=0.5):
        if (not 1 <= max_checks <= 12 or ttl_seconds <= 0 or max_candidates < 20
                or refresh_max_requests < 1 or refresh_timeout_seconds <= 0):
            raise ValueError("代理池容量或校验参数无效")
        timings = (warmup_seconds, target_cooldown_seconds, transport_cooldown_seconds,
                   transport_window_seconds, route_request_interval_seconds)
        if any(not math.isfinite(value) or value < 0 for value in timings):
            raise ValueError("出口调度参数无效")
        self.shared = bool(shared)
        self.include_direct = bool(include_direct)
        self._warmup_seconds = float(warmup_seconds)
        self._warmup_deadline = None
        self._warmup_done = False
        self._warmup_seen = set()
        self._target_cooldown = float(target_cooldown_seconds)
        self._transport_cooldown = float(transport_cooldown_seconds)
        self._transport_window = float(transport_window_seconds)
        self._route_request_interval = float(route_request_interval_seconds)
        self._round_robin = 0
        self.source = "imported" if imported_candidates is not None else "public"
        self._imported = {item["endpoint"]: dict(item) for item in (imported_candidates or [])}
        self._imported_cursor = 0
        self._subscription_cursor = 0
        self._subscription_refill = False
        self._fetcher = self._fetch_imported if self.source == "imported" else (fetcher or fetch_candidates)
        self._validator = validator
        self._max_checks = max_checks
        self._ttl = float(ttl_seconds)
        self._request_interval = max(1.0, float(request_interval_seconds))
        self._capacity = int(max_candidates)
        self._condition = threading.Condition(threading.RLock())
        self._available = OrderedDict()
        self._routes = OrderedDict()
        self._retired = {}
        self._controls = {}
        self._generations = {endpoint: 0 for endpoint in self._imported}
        self._suspended = set()
        self._auth_blocked = set()
        self._transport_failures = {}
        self._recheck_after = {}
        self._leases = {}
        self._pending = deque()
        self._pending_set = set()
        self._checking = {}
        self._checking_generations = {}
        self._failed = OrderedDict()
        self._failure_registry = failure_registry if failure_registry is not None else ProxyFailureRegistry()
        self._waiting = {}
        self._thread = None
        self._executor = None
        self._closed = False
        self._stop_event = threading.Event()
        self._refresh_max_requests = int(refresh_max_requests)
        self._refresh_timeout = float(refresh_timeout_seconds)
        self._refresh_endpoints = set()
        self._refresh_seen = set()
        self._refresh_deadline = 0.0
        self._refresh_empty_batches = 0
        self._refresh_source_done = True
        self._refresh = {"active": False, "target": 50, "scheduled": 0, "checked": 0,
                         "passed": 0, "failed": 0, "source_requests": 0,
                         "skipped_failed": 0, "skipped_existing": 0,
                         "state": "idle", "detail": "尚未手动检查代理"}
        self._fetching = False
        self._next_fetch = 0.0
        self._fetch_failures = 0
        self._fetch_error = ""
        self._fetch_count = 0
        self._checks_total = 0
        self._consecutive_check_failures = 0
        self._last_error = ""
        if self.include_direct:
            control = self._control_for("direct")
            self._direct = ProxyLease("direct", "", 0, 0, "", kind="direct", route_id="direct", control=control)
        else:
            self._direct = None

    def _control_for(self, route_id):
        if route_id not in self._controls:
            self._controls[route_id] = RouteControl(request_interval_seconds=self._route_request_interval)
        return self._controls[route_id]

    @staticmethod
    def _record_url(endpoint, record):
        if record is None:
            return f"http://{endpoint}"
        auth = ""
        if record.get("username"):
            auth = f"{quote(record['username'], safe='')}:{quote(record['password'], safe='')}@"
        return f"{record['protocol']}://{auth}{endpoint}"

    def _proxy_url_for(self, endpoint):
        return self._record_url(endpoint, self._imported.get(endpoint))

    def _expired(self, endpoint):
        expiry = self._imported.get(endpoint, {}).get("expires_at")
        return bool(expiry and datetime.fromisoformat(expiry).timestamp() <= time.time())

    def _fetch_imported(self, count):
        with self._condition:
            endpoints = [endpoint for endpoint in self._imported if not self._expired(endpoint)]
            if not endpoints:
                return []
            result = [endpoints[(self._imported_cursor + index) % len(endpoints)]
                      for index in range(min(count, len(endpoints)))]
            self._imported_cursor = (self._imported_cursor + len(result)) % len(endpoints)
            return result

    def candidate_status(self, endpoint):
        with self._condition:
            if any(lease.endpoint == endpoint for lease in self._leases.values()):
                return "assigned"
            if self._expired(endpoint):
                return "expired"
            if endpoint in self._checking:
                return "checking"
            if endpoint in self._pending_set:
                return "queued"
            if endpoint in self._available:
                return "available"
            if self._failure_registry.contains(endpoint):
                return "failed"
            return "unchecked"

    def _start_locked(self):
        if self._thread is None:
            executor = ThreadPoolExecutor(max_workers=self._max_checks, thread_name_prefix="proxy-check")
            thread = threading.Thread(target=self._run, name="proxy-source", daemon=True)
            self._executor = executor
            try:
                thread.start()
            except Exception:
                executor.shutdown(wait=False, cancel_futures=True)
                self._executor = None
                raise
            self._thread = thread

    def _prune_locked(self):
        now = time.monotonic()
        for endpoint, candidate in list(self._available.items()):
            if now - candidate.verified_monotonic >= self._ttl or self._expired(endpoint):
                del self._available[endpoint]
                active = any(item.route_id == candidate.route_id and item.generation == candidate.generation
                             for item in self._leases.values())
                if not active:
                    self._routes.pop(endpoint, None)
                elif (self.shared and not self._closed and not self._expired(endpoint) and
                      not self._failure_registry.contains(endpoint) and
                      endpoint not in self._checking and endpoint not in self._pending_set):
                    self._pending.append(endpoint)
                    self._pending_set.add(endpoint)
                    self._condition.notify_all()
        self._cleanup_controls_locked()

    def _warmup_ready_locked(self):
        if not self.shared or self._warmup_done:
            return True
        if self._warmup_deadline is None:
            # A prepared healthy pool is immediately usable.
            if self._available:
                self._warmup_done = True
                return True
            self._warmup_deadline = time.monotonic() + self._warmup_seconds
        source_done = (bool(self._warmup_seen) if self.source == "public" else
                       all(ep in self._warmup_seen or self._failure_registry.contains(ep) or self._expired(ep)
                           for ep in self._imported))
        if (time.monotonic() >= self._warmup_deadline or
                (source_done and not self._pending and not self._checking and not self._fetching)):
            self._warmup_done = True
        return self._warmup_done

    def _choose_locked(self):
        if not self._warmup_ready_locked():
            return None
        candidates = list(self._available.values())
        if self._direct is not None:
            candidates.append(self._direct)
        candidates = [item for item in candidates
                      if (item.endpoint, item.generation) not in self._suspended
                      and item.control.snapshot()["cooldown_seconds"] <= 0]
        if not candidates:
            return None
        if not self.shared:
            return candidates[0]
        counts = {item.route_id: 0 for item in candidates}
        for lease in self._leases.values():
            if lease.route_id in counts:
                counts[lease.route_id] += 1
        minimum = min(counts.values())
        # Rotate over all exits, including tied exits with active older generations.
        for offset in range(len(candidates)):
            index = (self._round_robin + offset) % len(candidates)
            item = candidates[index]
            if counts[item.route_id] == minimum:
                self._round_robin = (index + 1) % len(candidates)
                return item

    def get(self, rid):
        with self._condition:
            return self._leases.get(str(rid))

    def revalidate(self, rid, lease):
        """Keep a lease for the next connection, or release only its old token.

        Call between connections/metadata attempts, never on a healthy socket.
        An ordinary cooldown keeps the binding and waits at RouteControl.
        """
        with self._condition:
            current = self._leases.get(str(rid))
            if self._closed or current is None or current is not lease:
                return False
            candidate = self._routes.get(current.endpoint)
            usable = current.kind == "direct" or (
                candidate is not None and candidate.generation == current.generation
                and (current.endpoint, current.generation) not in self._suspended
                and not self._expired(current.endpoint)
                and (self.source != "imported" or current.endpoint in self._imported))
            if not usable:
                self.release(rid, current)
            return usable

    def acquire(self, rid, stop_event):
        rid = str(rid)
        with self._condition:
            if self._closed or stop_event.is_set():
                return None
            self._start_locked()
            self._waiting[rid] = self._waiting.get(rid, 0) + 1
            try:
                while not self._closed and not stop_event.is_set():
                    existing = self._leases.get(rid)
                    if existing is not None:
                        return existing
                    self._prune_locked()
                    candidate = self._choose_locked()
                    if candidate is not None:
                        if not self.shared and candidate.kind != "direct":
                            self._available.pop(candidate.endpoint, None)
                        lease = replace(candidate, rid=rid, acquired_at=_iso())
                        self._leases[rid] = lease
                        return lease
                    self._condition.notify_all()
                    self._condition.wait(0.1)
                return None
            finally:
                self._waiting[rid] -= 1
                if not self._waiting[rid]:
                    del self._waiting[rid]
                self._condition.notify_all()

    def release(self, rid, lease=None):
        with self._condition:
            current = self._leases.get(str(rid))
            if current is None or (lease is not None and current is not lease):
                return False
            del self._leases[str(rid)]
            is_current = (current.kind == "proxy" and self._routes.get(current.endpoint) is not None and
                          self._routes[current.endpoint].generation == current.generation)
            if (is_current and not self._closed and not self._expired(current.endpoint)
                    and (current.endpoint, current.generation) not in self._suspended
                    and time.monotonic() - current.verified_monotonic < self._ttl):
                self._available[current.endpoint] = self._routes[current.endpoint]
            self._cleanup_retired_locked()
            if is_current and current.endpoint not in self._available and not any(
                    item.endpoint == current.endpoint for item in self._leases.values()):
                self._routes.pop(current.endpoint, None)
            self._condition.notify_all()
            return True

    def _cleanup_retired_locked(self):
        used = {(item.route_id, item.generation) for item in self._leases.values()}
        self._retired = {key: item for key, item in self._retired.items() if key in used}
        self._cleanup_controls_locked()

    def _cleanup_controls_locked(self):
        used = ({item.route_id for item in self._routes.values()} |
                {item.route_id for item in self._retired.values()} |
                {item.route_id for item in self._leases.values()} |
                {f"proxy:{endpoint}" for endpoint in set(self._checking) | self._pending_set})
        if self._direct is not None:
            used.add("direct")
        for route_id in list(self._controls):
            if route_id not in used:
                self._controls.pop(route_id, None)
        for mapping in (self._transport_failures, self._recheck_after):
            for key in list(mapping):
                if key[0] not in used:
                    mapping.pop(key, None)

    def _occupied_endpoints_locked(self):
        return (set(self._routes) | set(self._checking) | self._pending_set |
                {item.endpoint for item in self._leases.values() if item.kind == "proxy"})

    def _refill_subscription_locked(self):
        """Admit a bounded slice of saved candidates; leave the rest in config."""
        if not self._subscription_refill or self._closed or not self._imported:
            return
        occupied = self._occupied_endpoints_locked()
        outstanding = set(self._routes) | set(self._checking) | self._pending_set
        hosts = {endpoint.rsplit(":", 1)[0] for endpoint in occupied}
        endpoints = list(self._imported)
        start = self._subscription_cursor % len(endpoints)
        for offset in range(len(endpoints)):
            index = (start + offset) % len(endpoints)
            endpoint = endpoints[index]
            if endpoint in outstanding or self._failure_registry.contains(endpoint) or self._expired(endpoint):
                continue
            host = endpoint.rsplit(":", 1)[0]
            if endpoint not in occupied and (len(occupied) >= self._capacity or host in hosts):
                continue
            # Replacing credentials for an already occupied endpoint does not
            # consume another exit slot; its old sockets may still be draining.
            self._pending.append(endpoint)
            self._pending_set.add(endpoint)
            self._warmup_seen.add(endpoint)
            occupied.add(endpoint)
            outstanding.add(endpoint)
            hosts.add(host)
            self._subscription_cursor = (index + 1) % len(endpoints)

    def update_imported_candidates(self, records):
        """Replace subscription records without changing existing socket leases."""
        updated = {normalize_endpoint(item["endpoint"]): dict(item) for item in records}
        with self._condition:
            if self._closed or self.source != "imported":
                return False
            def expired(record):
                value = record.get("expires_at") if record is not None else None
                return bool(value and datetime.fromisoformat(value).timestamp() <= time.time())

            def route_changed(endpoint):
                before, after = self._imported.get(endpoint), updated.get(endpoint)
                if before is None or after is None:
                    return True
                return (any(before.get(key) != after.get(key) for key in ("protocol", "username", "password"))
                        or (expired(before) and not expired(after)))

            changed = {ep for ep in set(self._imported) | set(updated) if route_changed(ep)}
            self._imported = updated
            for ep in changed:
                self._generations[ep] = self._generations.get(ep, 0) + 1
                old = self._routes.pop(ep, None)
                if old is not None:
                    self._retired[(old.route_id, old.generation)] = old
                self._available.pop(ep, None)
                self._pending = deque(item for item in self._pending if item != ep)
                self._pending_set.discard(ep)
                self._record_refresh_result_locked(ep, False)
                if ep in updated:
                    # Explicit replacement may fix credentials previously rejected.
                    self._failure_registry.discard(ep)
                    self._failed.pop(ep, None)
                    if not self._expired(ep):
                        self._subscription_refill = True
            self._cleanup_retired_locked()
            self._prune_locked()
            self._refill_subscription_locked()
            if self._pending:
                self._start_locked()
            self._condition.notify_all()
            return True

    def _failure_locked(self, endpoint, error, status_code=0, latency_ms=0):
        self._failure_registry.add(endpoint)
        previous = self._failed.get(endpoint, {})
        attempts = min(previous.get("attempts", 0) + 1, 5)
        self._failed[endpoint] = {"next_check": time.monotonic() + min(300, 30 * 2 ** (attempts - 1)),
                                 "attempts": attempts, "status_code": status_code, "error": error,
                                 "latency_ms": latency_ms, "checked_at": _iso()}
        self._failed.move_to_end(endpoint)
        while len(self._failed) > 2048:
            self._failed.popitem(last=False)
        self._last_error = error

    def invalidate(self, rid, lease=None):
        with self._condition:
            current = self._leases.get(str(rid))
            if current is None or (lease is not None and current is not lease):
                return False
            del self._leases[str(rid)]
            if current.kind != "direct":
                self._available.pop(current.endpoint, None)
                self._suspended.add((current.endpoint, current.generation))
                self._failure_locked(current.endpoint, "直播连接失败，本次应用运行期间跳过此代理")
                if not any(item.endpoint == current.endpoint for item in self._leases.values()):
                    self._routes.pop(current.endpoint, None)
            self._cleanup_retired_locked()
            self._condition.notify_all()
            return True

    def report_failure(self, rid, lease, kind, status_code=0):
        with self._condition:
            current = self._leases.get(str(rid))
            if self._closed or current is None or current is not lease:
                return False
            if kind == "target_http":
                if status_code in (444, 429):
                    current.control.cooldown(self._target_cooldown)
                self._condition.notify_all()
                return True
            if kind == "proxy_auth":
                if current.kind != "direct":
                    self._suspended.add((current.endpoint, current.generation))
                    self._auth_blocked.add((current.endpoint, current.generation))
                    candidate = self._available.get(current.endpoint)
                    if candidate is not None and candidate.generation == current.generation:
                        self._available.pop(current.endpoint, None)
                    if self._generations.get(current.endpoint, 0) == current.generation:
                        self._failure_locked(current.endpoint, AUTH_FAILURE_MESSAGE, 407)
                self.release(rid, current)
                return False
            if kind == "transport":
                if not self.revalidate(rid, current):
                    # Retired/expired/suspended routes keep healthy sockets, but
                    # never trap a failed room on an obsolete binding.
                    return False
                now = time.monotonic()
                key = (current.route_id, current.generation)
                failures = self._transport_failures.setdefault(key, {})
                failures = {room: when for room, when in failures.items() if now - when < self._transport_window}
                failures[str(rid)] = now
                self._transport_failures[key] = failures
                if len(failures) >= 3 and now >= self._recheck_after.get(key, 0):
                    current.control.cooldown(self._transport_cooldown)
                    self._recheck_after[key] = now + max(self._transport_window, self._transport_cooldown)
                    if (current.kind != "direct" and self._generations.get(current.endpoint, 0) == current.generation
                            and (self.source != "imported" or current.endpoint in self._imported)
                            and current.endpoint not in self._checking and current.endpoint not in self._pending_set):
                        self._pending.append(current.endpoint)
                        self._pending_set.add(current.endpoint)
                        self._start_locked()
                self._condition.notify_all()
                return True
            raise ValueError("出口失败类型无效")

    def refresh(self):
        with self._condition:
            if self._closed or self._refresh["active"]:
                return False
            self._start_locked()
            target = min(50, len(self._imported)) if self.source == "imported" else 50
            self._refresh = {"active": True, "target": target, "scheduled": 0, "checked": 0,
                             "passed": 0, "failed": 0, "source_requests": 0,
                             "skipped_failed": 0, "skipped_existing": 0,
                             "state": "running", "detail": f"正在分批获取并检查 {target} 个新候选"}
            self._refresh_endpoints.clear()
            self._refresh_seen.clear()
            self._refresh_deadline = time.monotonic() + self._refresh_timeout
            self._refresh_empty_batches = 0
            self._refresh_source_done = False
            if target == 0:
                self._end_refresh_source_locked("尚未导入代理，请先导入")
            self._condition.notify_all()
            return True

    def _end_refresh_source_locked(self, detail=""):
        self._refresh_source_done = True
        if detail:
            self._refresh["detail"] = detail
        self._finish_refresh_locked()

    def _finish_refresh_locked(self):
        if not self._refresh["active"] or not self._refresh_source_done or self._refresh_endpoints:
            return
        self._refresh["active"] = False
        complete = self._refresh["checked"] == self._refresh["target"]
        self._refresh["state"] = "completed" if complete else "partial"
        if complete:
            self._refresh["detail"] = f"已检查 {self._refresh['target']} 个新候选，通过 {self._refresh['passed']} 个"
        else:
            reason = self._refresh["detail"]
            self._refresh["detail"] = f"本批只检查到 {self._refresh['checked']}/{self._refresh['target']} 个新候选；{reason}"

    def _record_refresh_result_locked(self, endpoint, okay):
        if endpoint in self._refresh_endpoints:
            self._refresh_endpoints.remove(endpoint)
            self._refresh["checked"] += 1
            self._refresh["passed" if okay else "failed"] += 1
        self._finish_refresh_locked()

    def _check(self, endpoint, proxy_url=None, control=None):
        started = time.monotonic()
        release = None
        try:
            if control is not None:
                release = control.acquire_request(self._stop_event)
                if release is None:
                    return False, 0, 0, "代理检查已取消"
            result = (self._validator(endpoint) if self._validator is not None else
                      validate_proxy(endpoint, proxy_url=proxy_url or self._proxy_url_for(endpoint)))
            if not isinstance(result, dict):
                raise ValueError("invalid probe result")
            code = int(result.get("status_code", 0))
            # A redirect or error response is not a successful homepage probe.
            okay = bool(result.get("ok")) and code == 200
            latency = max(0.0, float(result.get("latency_ms", (time.monotonic() - started) * 1000)))
            error = "" if okay else AUTH_FAILURE_MESSAGE if code == 407 else f"目标服务返回 HTTP {code}" if code else "代理健康检查失败"
            return okay, code, latency, error
        except Exception as exc:
            if _is_proxy_auth_failure(exc):
                return False, 0, round((time.monotonic() - started) * 1000, 1), AUTH_FAILURE_MESSAGE
            error = f"代理健康检查失败（{type(exc).__name__}）"
            if isinstance(exc, requests.exceptions.SSLError):
                detail = str(exc)
                error = "经代理访问目标站点时，TLS 连接失败"
                if "UNEXPECTED_EOF" in detail:
                    error = "经代理访问目标站点时，TLS 连接被提前关闭"
                elif "WRONG_VERSION_NUMBER" in detail:
                    error = "代理隧道未返回有效的 TLS 响应"
                elif "CERTIFICATE_VERIFY_FAILED" in detail:
                    error = "经代理访问目标站点时，TLS 证书验证失败"
            return False, 0, round((time.monotonic() - started) * 1000, 1), error
        finally:
            if release is not None:
                release()

    def _checked(self, endpoint, future, generation=0, record=None, proxy_url=None):
        try:
            okay, code, latency, error = future.result()
        except Exception as exc:
            okay, code, latency, error = False, 0, 0, f"代理校验取消或失败（{type(exc).__name__}）"
        with self._condition:
            if self._checking.get(endpoint) is future:
                self._checking.pop(endpoint, None)
                self._checking_generations.pop(endpoint, None)
            self._checks_total += 1
            if (self._closed or generation != self._generations.get(endpoint, 0) or
                    (self.source == "imported" and endpoint not in self._imported)):
                self._condition.notify_all()
                return
            if (endpoint, generation) in self._auth_blocked:
                okay, error = False, AUTH_FAILURE_MESSAGE
            if okay and self._expired(endpoint):
                okay, error = False, "导入代理已到期"
            self._record_refresh_result_locked(endpoint, okay)
            if not self._closed:
                if okay and not self._expired(endpoint):
                    self._consecutive_check_failures = 0
                    self._failed.pop(endpoint, None)
                    record = record or {}
                    route_id = f"proxy:{endpoint}"
                    candidate = ProxyLease(endpoint, proxy_url or self._proxy_url_for(endpoint), code, latency, _iso(),
                                                           verified_monotonic=time.monotonic(),
                                                           protocol=record.get("protocol", "http"),
                                                           auth_present=bool(record.get("username")),
                                                           route_id=route_id, control=self._control_for(route_id),
                                                           generation=generation)
                    self._suspended.discard((endpoint, generation))
                    self._routes[endpoint] = candidate
                    if self.shared or not any(item.endpoint == endpoint for item in self._leases.values()):
                        self._available[endpoint] = candidate
                else:
                    self._available.pop(endpoint, None)
                    self._suspended.add((endpoint, generation))
                    if not any(item.endpoint == endpoint for item in self._leases.values()):
                        self._routes.pop(endpoint, None)
                    self._failure_locked(endpoint, error, code, latency)
                    self._consecutive_check_failures += 1
                    if self._consecutive_check_failures % 20 == 0:
                        delay = min(60, 2 ** min(6, self._consecutive_check_failures // 20))
                        self._next_fetch = max(self._next_fetch, time.monotonic() + delay)
            self._finish_refresh_locked()
            self._cleanup_controls_locked()
            self._condition.notify_all()

    def _run(self):
        while True:
            with self._condition:
                if self._closed:
                    return
                self._prune_locked()
                self._refill_subscription_locked()
                if (self._refresh["active"] and not self._refresh_source_done
                        and time.monotonic() >= self._refresh_deadline):
                    self._end_refresh_source_locked("本次获取候选已到时间上限，可稍后再次检查")
                pending_count = len(self._pending)
                while pending_count and self._pending and len(self._checking) < self._max_checks:
                    pending_count -= 1
                    endpoint = self._pending.popleft()
                    if endpoint in self._checking:
                        self._pending.append(endpoint)
                        continue
                    self._pending_set.remove(endpoint)
                    generation = self._generations.get(endpoint, 0)
                    record = dict(self._imported[endpoint]) if endpoint in self._imported else None
                    proxy_url = self._record_url(endpoint, record)
                    try:
                        control = self._control_for(f"proxy:{endpoint}")
                        future = self._executor.submit(self._check, endpoint, proxy_url, control)
                    except RuntimeError:
                        self._failure_locked(endpoint, "系统无法启动代理校验任务")
                        self._checks_total += 1
                        self._record_refresh_result_locked(endpoint, False)
                        continue
                    self._checking[endpoint] = future
                    self._checking_generations[endpoint] = generation
                    future.add_done_callback(lambda result, endpoint=endpoint, generation=generation,
                                             record=record, proxy_url=proxy_url:
                                             self._checked(endpoint, result, generation, record, proxy_url))
                outstanding = len(self._available) + len(self._checking) + len(self._pending)
                demand = sum(rid not in self._leases for rid in self._waiting)
                manual_fetch = self._refresh["active"] and not self._refresh_source_done
                occupied_count = len(self._occupied_endpoints_locked())
                if (manual_fetch and not self._checking and not self._pending
                        and occupied_count >= self._capacity):
                    self._end_refresh_source_locked("代理池容量已满，未替换正在使用的代理")
                    manual_fetch = False
                warmup_fetch = (self.shared and self._warmup_deadline is not None and
                                not self._warmup_ready_locked() and
                                (not self._warmup_seen if self.source == "public" else
                                 any(ep not in self._warmup_seen and not self._failure_registry.contains(ep)
                                     and not self._expired(ep) for ep in self._imported)))
                automatic_fetch = (demand > outstanding if not self.shared else
                                   bool(demand and not self._available and self._direct is None))
                need_fetch = manual_fetch or warmup_fetch or automatic_fetch
                now = time.monotonic()
                if not need_fetch or occupied_count >= self._capacity or now < self._next_fetch:
                    self._condition.wait(0.1 if need_fetch else 0.5)
                    continue
                self._fetching = True
                self._fetch_count += 1
                self._next_fetch = now + self._request_interval
                request_count = 20
                if manual_fetch:
                    self._refresh["source_requests"] += 1
                    request_count = min(20, self._refresh["target"] - self._refresh["scheduled"])
            error, candidates = "", []
            try:
                candidates = self._fetcher(request_count)
                if not isinstance(candidates, list):
                    raise ProxyPoolError("代理源候选列表格式无效")
            except Exception as exc:
                error = str(exc) if isinstance(exc, ProxyPoolError) else f"获取代理失败（{type(exc).__name__}）"
            with self._condition:
                self._fetching = False
                if self._closed:
                    self._condition.notify_all()
                    return
                occupied = self._occupied_endpoints_locked()
                occupied_hosts = {endpoint.rsplit(":", 1)[0] for endpoint in occupied}
                added = 0
                for candidate in candidates[:request_count]:
                    try:
                        endpoint = normalize_endpoint(candidate)
                    except (ValueError, TypeError):
                        continue
                    if self.source == "imported" and endpoint not in self._imported:
                        continue
                    self._warmup_seen.add(endpoint)
                    host = endpoint.rsplit(":", 1)[0]
                    if self._failure_registry.contains(endpoint):
                        if manual_fetch:
                            self._refresh["skipped_failed"] += 1
                        continue
                    if host in occupied_hosts or (manual_fetch and endpoint in self._refresh_seen):
                        if manual_fetch:
                            self._refresh["skipped_existing"] += 1
                        continue
                    if len(occupied) >= self._capacity:
                        break
                    self._pending.append(endpoint)
                    self._pending_set.add(endpoint)
                    occupied.add(endpoint)
                    occupied_hosts.add(host)
                    added += 1
                    if manual_fetch:
                        self._refresh_seen.add(endpoint)
                        self._refresh_endpoints.add(endpoint)
                        self._refresh["scheduled"] += 1
                if not added:
                    self._fetch_failures = min(self._fetch_failures + 1, 6)
                    self._fetch_error = error or ("已导入代理暂未提供新的候选，请检查地址、到期时间及失败记录" if self.source == "imported"
                                                 else "代理源暂未提供新的候选，稍后重试")
                    self._last_error = self._fetch_error
                    self._next_fetch = max(self._next_fetch, time.monotonic() + min(60, 2 ** self._fetch_failures))
                else:
                    self._fetch_failures = 0
                    self._fetch_error = ""
                if manual_fetch:
                    self._refresh_empty_batches = 0 if added else self._refresh_empty_batches + 1
                    if self._refresh["scheduled"] >= self._refresh["target"]:
                        self._end_refresh_source_locked(f"已取得 {self._refresh['target']} 个新候选，正在完成健康检查")
                    elif self._refresh_empty_batches >= 3:
                        self._end_refresh_source_locked("代理源连续 3 批未提供可检查的新候选")
                    elif self._refresh["source_requests"] >= self._refresh_max_requests:
                        self._end_refresh_source_locked("已到本批请求次数上限，代理源候选不足")
                    elif time.monotonic() >= self._refresh_deadline:
                        self._end_refresh_source_locked("本次获取候选已到时间上限，可稍后再次检查")
                self._condition.notify_all()

    def snapshot(self):
        with self._condition:
            self._prune_locked()
            active_endpoints = set(self._available) | set(self._checking) | self._pending_set | {
                item.endpoint for item in self._leases.values() if item.kind == "proxy"}
            failed_count = self._failure_registry.count()
            leases = [{"rid": rid, "proxy": item.endpoint, "status": "assigned", "status_code": item.status_code,
                       "latency_ms": item.latency_ms, "verified_at": item.verified_at, "acquired_at": item.acquired_at,
                       "kind": item.kind, "route_id": item.route_id}
                      for rid, item in sorted(self._leases.items())]
            routes = []
            # A credential generation is not a new physical exit. Show one row
            # and one set of counters even while old sockets drain.
            current_ids = {item.route_id for item in self._routes.values()}
            candidates = {item.route_id: item for item in self._retired.values()}
            candidates.update({item.route_id: item for item in self._routes.values()})
            if self._direct is not None and not self._closed:
                candidates[self._direct.route_id] = self._direct
                current_ids.add(self._direct.route_id)
            for candidate in candidates.values():
                counters = candidate.control.snapshot()
                state = ("draining" if candidate.route_id not in current_ids or
                         (candidate.kind == "proxy" and self._expired(candidate.endpoint)) else "suspended"
                         if (candidate.endpoint, candidate.generation) in self._suspended else
                         "cooldown" if counters["cooldown_seconds"] > 0 else "ready")
                routes.append({"kind": candidate.kind, "route_id": candidate.route_id,
                               "endpoint": candidate.endpoint, "state": state,
                               "assigned_tasks": sum(item.route_id == candidate.route_id for item in self._leases.values()),
                               "connecting": counters["connecting"],
                               "request_inflight": counters["request_inflight"],
                               "cooldown_seconds": counters["cooldown_seconds"]})
            return {"source": self.source, "total": len(active_endpoints) + failed_count,
                    "available": sum(item.control.snapshot()["cooldown_seconds"] <= 0
                                     for item in self._available.values()),
                    "checking": len(self._checking), "pending": len(self._pending),
                    "assigned": len(leases), "failed": failed_count, "waiting": len(self._waiting),
                    "fetching": self._fetching, "fetch_count": self._fetch_count, "checks_total": self._checks_total,
                    "consecutive_check_failures": self._consecutive_check_failures,
                    "fetch_error": self._fetch_error, "last_error": self._last_error, "leases": leases,
                    "closed": self._closed, "max_checks": self._max_checks, "ttl_seconds": self._ttl,
                    "refresh": dict(self._refresh), "shared": self.shared,
                    "include_direct": self.include_direct, "routes": routes}

    def shutdown(self, timeout=5):
        deadline = time.monotonic() + max(0, timeout)
        with self._condition:
            self._closed = True
            self._stop_event.set()
            if self._refresh["active"]:
                self._refresh.update(active=False, state="cancelled", detail="代理池已关闭，本批检查已取消")
            self._pending.clear()
            self._pending_set.clear()
            self._available.clear()
            self._leases.clear()
            self._routes.clear()
            self._retired.clear()
            for control in self._controls.values():
                control.close()
            futures = list(self._checking.values())
            thread, executor = self._thread, self._executor
            self._condition.notify_all()
        if executor is not None:
            executor.shutdown(wait=False, cancel_futures=True)
        if thread is not None and thread is not threading.current_thread():
            thread.join(max(0, deadline - time.monotonic()))
        if futures:
            wait(futures, timeout=max(0, deadline - time.monotonic()))
        return (thread is None or not thread.is_alive()) and all(future.done() for future in futures)

    close = shutdown
