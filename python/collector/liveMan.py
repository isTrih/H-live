
# coding:utf-8

# @FileName:    liveMan.py
# @Time:        2024/1/2 21:51
# @Author:      bubu
# @Project:     douyinLiveWebFetcher

import json
import hashlib
import logging
import queue
import random
import re
import string
import ssl
import threading
import time
import urllib.parse
import weakref
from datetime import datetime, timedelta, timezone
from pathlib import Path

import requests
import websocket
from py_mini_racer import MiniRacer

from qingkan_native import decode_frame, hb_frame

LOGGER = logging.getLogger(__name__)
HTTP_TIMEOUT = (5, 10)
USER_AGENT = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) "
              "AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36")
SHANGHAI = timezone(timedelta(hours=8))
_SIGNATURE_SLOTS = threading.BoundedSemaphore(2)


class _HeartbeatHub:
    """A bounded shared heartbeat pool, independent of the number of rooms."""

    def __init__(self, workers=8):
        self._lock = threading.RLock()
        self._rooms = weakref.WeakKeyDictionary()
        self._queue = queue.Queue(maxsize=workers * 4)
        self._wake = threading.Event()
        self._started = False
        self._workers = workers

    def add(self, fetcher):
        with self._lock:
            self._rooms[fetcher] = [0, False]
            if not self._started:
                self._started = True
                for number in range(self._workers):
                    threading.Thread(target=self._work, name=f"heartbeat-worker-{number}", daemon=True).start()
                threading.Thread(target=self._schedule, name="heartbeat-scheduler", daemon=True).start()
        self._wake.set()

    def remove(self, fetcher):
        with self._lock:
            self._rooms.pop(fetcher, None)

    def contains(self, fetcher):
        with self._lock:
            return fetcher in self._rooms

    def _schedule(self):
        while True:
            self._wake.wait(0.25)
            self._wake.clear()
            now = time.monotonic()
            with self._lock:
                for fetcher, state in list(self._rooms.items()):
                    if fetcher._stop_event.is_set() or fetcher._heartbeat_stop.is_set():
                        self._rooms.pop(fetcher, None)
                        continue
                    if state[1] or state[0] > now:
                        continue
                    try:
                        self._queue.put_nowait(weakref.ref(fetcher))
                    except queue.Full:
                        break
                    state[1] = True

    def _work(self):
        while True:
            reference = self._queue.get()
            fetcher = reference()
            try:
                if fetcher is not None and self.contains(fetcher):
                    fetcher._sendHeartbeatOnce()
            except Exception as error:
                # One unexpected room failure must not shrink the shared pool.
                # Never include transport exception text (it may contain URLs).
                LOGGER.warning("共享心跳任务失败（%s）", type(error).__name__)
                if fetcher is not None:
                    self.remove(fetcher)
            finally:
                with self._lock:
                    if fetcher in self._rooms:
                        self._rooms[fetcher] = [time.monotonic() + 5, False]
                fetcher = None
                self._queue.task_done()


_HEARTBEATS = _HeartbeatHub()


class LiveMonitorError(RuntimeError):
    """An actionable connection or protocol error, without Cookie values."""

    def __init__(self, message, *, kind="transport", http_status=0, stage=""):
        super().__init__(message)
        self.kind = kind if kind in {"target_http", "proxy_auth", "transport", "cancelled"} else "transport"
        self.http_status = http_status if type(http_status) is int and 100 <= http_status <= 599 else 0
        self.stage = stage if stage in {"visitor", "room_info", "room_page", "websocket_handshake"} else ""


class LiveRoomUnavailableError(LiveMonitorError):
    """Room API rejection; not proof of offline, distinct from transport errors."""

    reason = "room_unavailable"

    def __init__(self):
        super().__init__("暂时无法取得直播间信息，可能已下播或访问受限，请等待重新确认", kind="target_http", stage="room_info")


class _MonitorStopped(LiveMonitorError):
    def __init__(self):
        super().__init__("监控已取消", kind="cancelled")


def _check_cancelled(stop_event):
    if stop_event is not None and stop_event.is_set():
        raise _MonitorStopped()


def _proxy_auth_error(error):
    # The pool's classifier traverses typed, bounded exception causes. Never
    # inspect a whole URL for numeric codes or return transport exception text.
    from proxy_pool import is_proxy_auth_failure
    if is_proxy_auth_failure(error):
        return True
    if isinstance(error, websocket.WebSocketProxyException):
        return bool(re.fullmatch(r"failed CONNECT via proxy status: 407", str(error)))
    from python_socks import ProxyError
    return isinstance(error, ProxyError) and str(error) in {
        "Username and password authentication failure", "No acceptable authentication methods were offered"}


def _validate_live_id(value):
    value = str(value).strip()
    if not re.fullmatch(r"[0-9]{1,32}", value):
        raise LiveMonitorError("直播间 ID 必须是 1 至 32 位数字")
    return value


def _cookie_value(cookie, name):
    for part in cookie.split(";"):
        key, separator, value = part.strip().partition("=")
        if separator and key == name:
            return value
    return ""


def _numeric_id(value):
    """Preserve long integer identifiers exactly; floats and names are not IDs."""
    if isinstance(value, bool) or not isinstance(value, (str, int)):
        return ""
    value = str(value).strip()
    return value if re.fullmatch(r"[0-9]+", value) and int(value) > 0 else ""


def _epoch_seconds(value):
    """Normalize server seconds/milliseconds without fabricating a local time."""
    if isinstance(value, bool) or not isinstance(value, (str, int, float)):
        return 0
    try:
        if isinstance(value, str) and not re.fullmatch(r"[0-9]+", value.strip()):
            return 0
        stamp = int(value)
        # Douyin sends 10-digit seconds or 13-digit milliseconds. Do not
        # reinterpret unrecognized micro/nanosecond values as plausible times.
        if stamp >= 100_000_000_000_000:
            return 0
        if stamp >= 100_000_000_000:
            stamp //= 1000
        if stamp <= 0:
            return 0
        datetime.fromtimestamp(stamp, tz=SHANGHAI)
        return stamp
    except (ValueError, TypeError, OverflowError, OSError):
        return 0


def _current_viewers(value):
    """Accept exact current counts only; display abbreviations are not counts."""
    if isinstance(value, bool) or not isinstance(value, (int, str)):
        return None
    if isinstance(value, str) and not re.fullmatch(r"[0-9]+", value.strip()):
        return None
    count = int(value)
    return count if 0 <= count <= 9_007_199_254_740_991 else None


def _http_get(session, url, *, traffic_control=None, stop_event=None, stage="", **kwargs):
    """Bound requests and expose useful failures without logging credentials."""
    release = None
    try:
        _check_cancelled(stop_event)
        if not url.startswith("https://"):
            raise LiveMonitorError("拒绝通过未加密 HTTP 发送直播请求")
        if traffic_control is not None:
            release = traffic_control.acquire_request(stop_event)
            if release is None:
                raise _MonitorStopped()
        _check_cancelled(stop_event)
        response = session.get(url, timeout=HTTP_TIMEOUT, allow_redirects=False, **kwargs)
        _check_cancelled(stop_event)
        response.raise_for_status()
        status = getattr(response, "status_code", 200)
        if isinstance(status, int) and 300 <= status < 400:
            raise LiveMonitorError("直播请求被重定向，已停止以保护登录凭据", kind="target_http", http_status=status, stage=stage)
        return response
    except requests.Timeout:
        _check_cancelled(stop_event)
        raise LiveMonitorError("抖音请求超时，请检查网络后重试", stage=stage) from None
    except requests.HTTPError as error:
        _check_cancelled(stop_event)
        status = getattr(error.response, "status_code", "未知")
        if status == 407:
            raise LiveMonitorError("代理认证失败，请检查账号、密码或供应商访问权限", kind="proxy_auth", http_status=407, stage=stage) from None
        raise LiveMonitorError(f"抖音请求失败：HTTP {status}", kind="target_http", http_status=status, stage=stage) from None
    except requests.RequestException as error:
        _check_cancelled(stop_event)
        if _proxy_auth_error(error):
            raise LiveMonitorError("代理认证失败，请检查账号、密码或供应商访问权限", kind="proxy_auth", stage=stage) from None
        raise LiveMonitorError(f"无法访问抖音（{type(error).__name__}）", stage=stage) from None
    finally:
        if release is not None:
            release()


def _proxy_url(proxy):
    if not proxy:
        return ""
    value = getattr(proxy, "proxy_url", proxy)
    if value == "" and getattr(proxy, "kind", None) == "direct":
        return ""
    if not isinstance(value, str):
        raise LiveMonitorError("代理租约格式无效")
    try:
        if re.search(r"[\x00-\x20\x7f]", value) or re.search(r"%(?![0-9a-fA-F]{2})", value):
            raise ValueError()
        parsed = urllib.parse.urlsplit(value)
        port = parsed.port
        if (parsed.scheme not in ("http", "socks5h") or not parsed.hostname or not port
                or parsed.path not in ("", "/") or parsed.query or parsed.fragment):
            raise ValueError()
        authority = parsed.hostname
        if ":" in authority:
            authority = f"[{authority}]"
        userinfo = ""
        if parsed.username is not None or parsed.password is not None:
            if not parsed.username or not parsed.password:
                raise ValueError()
            username = urllib.parse.unquote(parsed.username, errors="strict")
            password = urllib.parse.unquote(parsed.password, errors="strict")
            if any(re.search(r"[\x00-\x1f\x7f]", item) for item in (username, password)):
                raise ValueError()
            if parsed.scheme == "http":
                # requests uses Latin-1 while websocket-client uses UTF-8 for
                # Basic authentication. ASCII preserves exactly one identity.
                if not username.isascii() or not password.isascii() or ":" in username:
                    raise ValueError()
            elif any(len(item.encode("utf-8")) > 255 for item in (username, password)):
                raise ValueError()
            userinfo = (urllib.parse.quote(username, safe="") + ":"
                        + urllib.parse.quote(password, safe="") + "@")
        return f"{parsed.scheme}://{userinfo}{authority}:{port}"
    except (ValueError, UnicodeError):
        # Never include a parsed value or the original URL in diagnostics.
        raise LiveMonitorError("代理配置无效；支持 HTTP CONNECT 或 SOCKS5 远端解析，HTTP 认证需使用 ASCII 账号密码") from None


def _configure_session(session, proxy):
    url = _proxy_url(proxy)
    session.trust_env = False
    session.verify = True
    session.proxies = {"http": url, "https": url} if url else {}
    return url


def _websocket_proxy_options(proxy):
    url = _proxy_url(proxy)
    if url:
        parsed = urllib.parse.urlsplit(url)
        options = {"http_proxy_host": parsed.hostname, "http_proxy_port": parsed.port,
                   "proxy_type": parsed.scheme, "http_no_proxy": ["never-bypass.invalid"]}
        if parsed.username is not None:
            options["http_proxy_auth"] = (urllib.parse.unquote(parsed.username, errors="strict"),
                                          urllib.parse.unquote(parsed.password, errors="strict"))
        return options
    # websocket-client drops http_no_proxy when proxy_host is empty. Supply an
    # unreachable sentinel plus '*' to explicitly bypass all proxies; its
    # get_proxy_info returns before DNS and cannot consult environment proxies.
    return {"http_proxy_host": "direct.invalid", "http_proxy_port": 80,
            "proxy_type": "http", "http_no_proxy": ["*"]}


def fetch_room_info(rid, cookie="", proxy=None, *, traffic_control=None, stop_event=None):
    """Read public room metadata; errors are distinct from a confirmed offline room.

    The upstream endpoint is unofficial. An absent room/status is an error,
    rather than evidence that the broadcaster is offline. No stream URL is read.
    """
    rid = _validate_live_id(rid)
    _check_cancelled(stop_event)
    traffic_control = traffic_control if traffic_control is not None else getattr(proxy, "control", None)
    session = requests.Session()
    try:
        _configure_session(session, proxy)
        headers = {"User-Agent": USER_AGENT, "Referer": f"https://live.douyin.com/{rid}"}
        if cookie:
            headers["Cookie"] = cookie
        else:
            response = _http_get(session, "https://live.douyin.com/", headers=headers,
                                 traffic_control=traffic_control, stop_event=stop_event, stage="visitor")
            ttwid = response.cookies.get("ttwid") or session.cookies.get("ttwid")
            if not ttwid:
                raise LiveMonitorError("抖音未返回访客凭据，可能需要有效登录 Cookie")
            headers["Cookie"] = f"ttwid={ttwid}"
        params = {
            "aid": "6383", "app_name": "douyin_web", "live_id": "1",
            "device_platform": "web", "language": "zh-CN", "enter_from": "page_refresh",
            "cookie_enabled": "true", "screen_width": "1920", "screen_height": "1080",
            "browser_language": "zh-CN", "browser_platform": "Win32", "browser_name": "Chrome",
            "browser_version": "140.0.0.0", "web_rid": rid, "room_id_str": rid,
            "is_need_double_stream": "false", "msToken": generateMsToken(107),
        }
        response = _http_get(session, "https://live.douyin.com/webcast/room/web/enter/",
                             headers=headers, params=params, traffic_control=traffic_control,
                             stop_event=stop_event, stage="room_info")
        try:
            result = response.json()
        except (ValueError, TypeError):
            raise LiveMonitorError("抖音返回的直播间信息不是有效 JSON，可能遇到验证页面") from None
        if not isinstance(result, dict):
            raise LiveMonitorError("抖音返回的直播间信息格式无效，无法判断是否开播")
        if result.get("status_code", 0) not in (0, "0"):
            raise LiveRoomUnavailableError()
        data = result.get("data")
        rooms = data.get("data") if isinstance(data, dict) else None
        if not isinstance(rooms, list) or not rooms or not isinstance(rooms[0], dict):
            raise LiveMonitorError("未取得直播间数据，无法判断是否开播；请检查 ID 或登录状态")
        room = rooms[0]
        status = room.get("status")
        if type(status) not in (int, str) or status not in (1, 2, 3, 4, "1", "2", "3", "4"):
            raise LiveMonitorError("抖音返回未知直播状态，无法判断是否开播")
        user = data.get("user") or {}
        if not isinstance(user, dict):
            user = {}
        owner = room.get("owner") or {}
        if not isinstance(owner, dict):
            owner = {}
        anchor_id = next((identity for value in (
            owner.get("id_str"), owner.get("id"), user.get("id_str"), user.get("id"),
            room.get("owner_user_id_str"), room.get("owner_user_id"))
            if (identity := _numeric_id(value))), "")
        return {
            "nickname": str(user.get("nickname") or owner.get("nickname") or ""),
            "is_live": str(status) == "2",
            "title": str(room.get("title") or ""),
            "anchor_id": anchor_id,
            "room_id": _numeric_id(room.get("id_str")) or _numeric_id(room.get("id")),
            "viewer_count": next((count for key in ("user_count", "online_count", "user_count_str")
                if (count := _current_viewers(room.get(key))) is not None), None) if str(status) == "2" else None,
            # A generic create_time may mean room/account creation. Only the
            # explicit room start_time is accepted as a broadcast start time.
            "started_at": _epoch_seconds(room.get("start_time")),
        }
    finally:
        session.close()


def generateSignature(wss, script_file=None, *, stop_event=None):
    # Each MiniRacer owns a V8 isolate. Hundreds of simultaneous connections
    # must not construct hundreds of isolates before their signatures finish.
    while True:
        _check_cancelled(stop_event)
        if _SIGNATURE_SLOTS.acquire(timeout=0.1):
            break
    try:
        _check_cancelled(stop_event)
        return _generate_signature(wss, script_file)
    finally:
        _SIGNATURE_SLOTS.release()


def _generate_signature(wss, script_file=None):
    """Resolve the bundled script independently of the process working directory."""
    params = ("live_id,aid,version_code,webcast_sdk_version,"
              "room_id,sub_room_id,sub_channel_id,did_rule,"
              "user_unique_id,device_platform,device_type,ac,"
              "identity").split(',')
    wss_maps = dict(urllib.parse.parse_qsl(urllib.parse.urlparse(wss).query, keep_blank_values=True))
    tpl_params = [f"{i}={wss_maps.get(i, '')}" for i in params]
    param = ','.join(tpl_params)
    md5 = hashlib.md5()
    md5.update(param.encode())
    md5_param = md5.hexdigest()

    ctx = None
    try:
        script_path = Path(script_file) if script_file else Path(__file__).resolve().with_name("sign.js")
        script = script_path.read_text(encoding="utf-8")
        ctx = MiniRacer()
        ctx.eval(script, timeout_sec=5)
        signature = ctx.call("get_sign", md5_param, timeout_sec=5)
        if not isinstance(signature, str) or not signature:
            raise LiveMonitorError("直播间签名为空，无法建立连接")
        return signature
    except LiveMonitorError:
        raise
    except Exception as error:
        raise LiveMonitorError(f"生成直播间签名失败（{type(error).__name__}）") from None
    finally:
        if ctx is not None:
            close = getattr(ctx, "close", None)
            if callable(close):
                try:
                    close()
                except Exception:
                    LOGGER.warning("JavaScript 引擎资源关闭失败")


def generateMsToken(length=182):
    """
    产生请求头部cookie中的msToken字段，其实为随机的107位字符
    :param length:字符位数
    :return:msToken
    """
    random_str = ''
    base_str = string.ascii_letters + string.digits + '-_'
    _len = len(base_str) - 1
    for _ in range(length):
        random_str += base_str[random.randint(0, _len)]
    return random_str


class DouyinLiveWebFetcher:

    def __init__(self, live_id, on_chat_message=None, on_message=None, cookie=None, on_status=None, proxy=None, traffic_control=None):
        """
        直播间弹幕抓取对象
        :param live_id: 直播间的直播id
        :param cookie: 登录后的完整 Cookie，用于接收礼物等需要登录的消息
        """
        self.on_chat_message = on_chat_message
        self.on_message = on_message
        self.on_status = on_status
        self.cookie = cookie or ""
        self.__ttwid = None
        self.__room_id = None
        self.live_id = _validate_live_id(live_id)
        self.session = requests.Session()
        self.proxy_url = _configure_session(self.session, proxy)
        self.traffic_control = traffic_control if traffic_control is not None else getattr(proxy, "control", None)
        self.host = "https://www.douyin.com/"
        self.live_url = "https://live.douyin.com/"
        self.user_agent = USER_AGENT
        self.headers = {
            'User-Agent': self.user_agent
        }
        self.ws = None
        self._stop_event = threading.Event()
        self._heartbeat_stop = threading.Event()
        self._heartbeat_thread = None
        self._state_lock = threading.RLock()
        self._started = False
        self._running = False
        self._connected = False
        self._closed_reported = False
        self._last_error = None
        self._user_unique_id = str(random.SystemRandom().randrange(10**18, 10**19))

    def start(self):
        """Block until disconnected. A stopped fetcher must not be reused."""
        with self._state_lock:
            if self._started:
                raise LiveMonitorError("同一个监控实例不能重复启动，请新建实例")
            self._started = True
            self._running = True
        try:
            self._ensure_running()
            self._notify_status("connecting", "正在连接直播间")
            self._ensure_running()
            self._connectWebSocket()
            self._ensure_running()
            if self._last_error:
                raise self._last_error
            if not self._connected:
                raise LiveMonitorError("WebSocket 在握手成功前关闭")
        except _MonitorStopped:
            pass
        except Exception as error:
            if not self._stop_event.is_set():
                failure = self._record_error(error)
                raise failure from None
        finally:
            self._heartbeat_stop.set()
            _HEARTBEATS.remove(self)
            self._close_socket()
            self.session.close()
            with self._state_lock:
                self._running = False
            self._report_closed()

    def stop(self):
        """Cancel before/during a connection; in-flight HTTP finishes within its timeout."""
        with self._state_lock:
            self._stop_event.set()
            self._heartbeat_stop.set()
            running = self._running
        _HEARTBEATS.remove(self)
        self._close_socket()
        if not running:
            self.session.close()
            self._report_closed()

    def _ensure_running(self):
        if self._stop_event.is_set():
            raise _MonitorStopped()

    def _notify_status(self, state, detail=""):
        if self.on_status:
            try:
                self.on_status(state, detail)
            except Exception as error:
                LOGGER.warning("监控状态回调失败（%s）", type(error).__name__)

    def _report_closed(self):
        with self._state_lock:
            if self._closed_reported:
                return
            self._closed_reported = True
            detail = "已停止" if self._stop_event.is_set() else "连接已关闭"
            self._notify_status("closed", detail)

    def _record_error(self, error):
        if isinstance(error, LiveMonitorError):
            failure = error
        elif _proxy_auth_error(error):
            failure = LiveMonitorError("代理认证失败，请检查账号、密码或供应商访问权限", kind="proxy_auth", stage="websocket_handshake")
        elif (isinstance(error, websocket.WebSocketBadStatusException)
              and type(error.status_code) is int and 100 <= error.status_code <= 599):
            message = "代理认证失败，请检查账号、密码或供应商访问权限" if error.status_code == 407 else f"WebSocket 握手失败：HTTP {error.status_code}"
            failure = LiveMonitorError(message, kind="proxy_auth" if error.status_code == 407 else "target_http",
                                       http_status=error.status_code, stage="websocket_handshake")
        elif type(getattr(error, "status_code", None)) is int:
            failure = LiveMonitorError(f"WebSocket 已关闭（代码 {error.status_code}）")
        else:
            failure = LiveMonitorError(f"WebSocket 连接失败（{type(error).__name__}）", stage="" if self._connected else "websocket_handshake")
        with self._state_lock:
            if self._last_error is None:
                self._last_error = failure
                if not self._stop_event.is_set():
                    self._notify_status("error", str(failure))
        return self._last_error

    def _close_socket(self):
        ws = self.ws
        if ws is not None:
            ws.keep_running = False
            try:
                sock = getattr(ws, "sock", None)
                if sock is not None:
                    # Wake blocking receive/send without waiting for a peer's
                    # close handshake on the UI thread.
                    sock.abort()
                    sock.shutdown()
                ws.close(timeout=0)
            except Exception:
                LOGGER.debug("关闭 WebSocket 时发生错误", exc_info=False)

    def _request_cookie(self):
        if self.cookie:
            return self.cookie
        return f"ttwid={self.ttwid}; msToken={generateMsToken()}"

    @property
    def ttwid(self):
        """
        产生请求头部cookie中的ttwid字段，访问抖音网页版直播间首页可以获取到响应cookie中的ttwid
        :return: ttwid
        """
        self._ensure_running()
        if self.__ttwid:
            return self.__ttwid
        supplied = _cookie_value(self.cookie, "ttwid")
        if supplied:
            self.__ttwid = supplied
            return supplied
        headers = {
            "User-Agent": self.user_agent,
        }
        if self.cookie:
            headers["Cookie"] = self.cookie
        response = _http_get(self.session, self.live_url, headers=headers, traffic_control=self.traffic_control,
                             stop_event=self._stop_event, stage="visitor")
        self._ensure_running()
        self.__ttwid = response.cookies.get('ttwid') or self.session.cookies.get('ttwid')
        if not self.__ttwid:
            raise LiveMonitorError("抖音未返回访客凭据，可能需要有效登录 Cookie")
        return self.__ttwid

    @property
    def room_id(self):
        """
        根据直播间的地址获取到真正的直播间roomId，有时会有错误，可以重试请求解决
        :return:room_id
        """
        self._ensure_running()
        if self.__room_id:
            return self.__room_id
        url = self.live_url + self.live_id
        headers = {
            "User-Agent": self.user_agent,
            "Cookie": self._request_cookie(),
        }
        self._ensure_running()
        response = _http_get(self.session, url, headers=headers, traffic_control=self.traffic_control,
                             stop_event=self._stop_event, stage="room_page")
        self._ensure_running()
        page = response.text.replace('\\"', '"')
        match = re.search(r'"roomId"\s*:\s*"?(\d+)"?', page)
        if not match:
            raise LiveMonitorError("页面中没有直播间 roomId，可能已下播、ID 无效或需要登录验证")
        self.__room_id = match.group(1)
        return self.__room_id

    def _connectWebSocket(self):
        """
        连接抖音直播间websocket服务器，请求直播间数据
        """
        self._ensure_running()
        room_id = self.room_id
        self._ensure_running()
        now_ms = int(time.time() * 1000)
        wss = ("wss://webcast100-ws-web-lq.douyin.com/webcast/im/push/v2/?app_name=douyin_web"
               "&version_code=180800&webcast_sdk_version=1.0.14-beta.0"
               "&update_version_code=1.0.14-beta.0&compress=gzip&device_platform=web&cookie_enabled=true"
               "&screen_width=1536&screen_height=864&browser_language=zh-CN&browser_platform=Win32"
               "&browser_name=Mozilla"
               "&browser_version=5.0%20(Windows%20NT%2010.0;%20Win64;%20x64)%20AppleWebKit/537.36%20(KHTML,"
               "%20like%20Gecko)%20Chrome/126.0.0.0%20Safari/537.36"
               "&browser_online=true&tz_name=Asia/Shanghai"
               f"&cursor=d-1_u-1_fh-0_t-{now_ms}_r-1"
               f"&internal_ext=internal_src:dim|wss_push_room_id:{room_id}|wss_push_did:{self._user_unique_id}"
               f"|first_req_ms:{now_ms}|fetch_time:{now_ms}|seq:1|wss_info:0-{now_ms}-0-0|wrds_v:0"
               f"&host=https://live.douyin.com&aid=6383&live_id=1&did_rule=3&endpoint=live_pc&support_wrds=1"
               f"&user_unique_id={self._user_unique_id}&im_path=/webcast/im/fetch/&identity=audience"
               f"&need_persist_msg_count=15&insert_task_id=&live_reason=&room_id={room_id}&heartbeatDuration=0")

        signature = generateSignature(wss, stop_event=self._stop_event)
        self._ensure_running()
        wss += "&signature=" + urllib.parse.quote(signature, safe="")

        if self.cookie:
            cookie_str = self.cookie
        else:
            cookie_str = self._request_cookie()
        headers = {
            "cookie": cookie_str,
            'user-agent': self.user_agent,
        }
        def connection_headers():
            # WebSocketApp invokes this after creating its socket and before
            # connect(). This keeps timeouts per instance instead of changing
            # websocket-client's process-wide default for every broadcaster.
            self._ensure_running()
            if self.ws.sock is not None:
                self.ws.sock.settimeout(HTTP_TIMEOUT[1])
            return headers

        self._ensure_running()
        self.ws = websocket.WebSocketApp(wss,
                                         header=connection_headers,
                                         on_open=self._wsOnOpen,
                                         on_message=self._wsOnMessage,
                                         on_error=self._wsOnError,
                                         on_close=self._wsOnClose)
        self._ensure_running()
        failed = self.ws.run_forever(http_proxy_timeout=HTTP_TIMEOUT[0], reconnect=0,
                                     sslopt={"cert_reqs": ssl.CERT_REQUIRED, "check_hostname": True,
                                             "ca_certs": requests.certs.where()},
                                     **_websocket_proxy_options(self.proxy_url))
        if failed and not self._last_error and not self._stop_event.is_set():
            raise LiveMonitorError("WebSocket 连接异常终止")

    def _sendHeartbeatOnce(self):
        if self._heartbeat_stop.is_set() or self._stop_event.is_set():
            return
        try:
            heartbeat = hb_frame()
            self.ws.send(heartbeat, websocket.ABNF.OPCODE_PING)
        except Exception as error:
            if not self._stop_event.is_set() and not self._heartbeat_stop.is_set():
                self._record_error(error)
                self._heartbeat_stop.set()
                _HEARTBEATS.remove(self)
                self._close_socket()

    def _wsOnOpen(self, ws):
        """
        连接建立成功
        """
        with self._state_lock:
            if self._stop_event.is_set():
                self._close_socket()
                return
            self._connected = True
            self._notify_status("connected", "已连接，等待直播间消息")
            if self._stop_event.is_set():
                self._close_socket()
                return
            _HEARTBEATS.add(self)

    def _wsOnMessage(self, ws, message):
        """Decode one bounded frame in Rust, then deliver its ordered events."""
        if self._stop_event.is_set():
            return
        try:
            ack, events_json = decode_frame(message, self.__room_id or "")
            events = json.loads(events_json)
        except Exception as error:
            LOGGER.warning("解析消息包失败（%s）", type(error).__name__)
            return
        if ack is not None:
            ws.send(ack, websocket.ABNF.OPCODE_BINARY)
        for event in events:
            if self._stop_event.is_set():
                break
            try:
                self._deliver(event)
            except Exception as error:
                LOGGER.warning("处理直播消息失败（%s）", type(error).__name__)

    def _deliver(self, event):
        kind, payload = event["type"], event["payload"]
        try:
            if self.on_message:
                self.on_message(kind, payload)
            if kind == "chat" and self.on_chat_message:
                self.on_chat_message(payload.get("user_name", ""), payload.get("content", ""), payload.get("event_time", 0))
        finally:
            # A failed consumer must not keep an ended session alive.
            if kind == "control":
                self.stop()

    def _wsOnError(self, ws, error):
        if not self._stop_event.is_set():
            self._record_error(error)
        self._heartbeat_stop.set()
        _HEARTBEATS.remove(self)

    def _wsOnClose(self, ws, *args):
        self._heartbeat_stop.set()
        _HEARTBEATS.remove(self)
