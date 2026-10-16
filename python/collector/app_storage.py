"""Validated local settings, per-user paths and Windows-protected credentials."""

from __future__ import annotations

import base64
import ctypes
import json
import os
from pathlib import Path
import re
import sys
import tempfile
from urllib.parse import urlsplit
from typing import TypedDict


class ProxyPoolConfig(TypedDict):
    """Fixed public proxy source, with an explicit direct-connection option."""
    provider: str
    enabled: bool
    api_url: str
    protocol: str
    schema_version: int
    source: str
    include_direct: bool


def default_proxy_pool() -> ProxyPoolConfig:
    return {"provider": "scdn", "enabled": True, "api_url": "https://proxy.scdn.io/api/get_proxy.php",
            "protocol": "https", "schema_version": 1, "source": "public", "include_direct": True}


class ConfigError(RuntimeError):
    """A configuration problem that can safely be displayed to the user."""


def app_data_dir() -> Path:
    """Keep settings and archives out of the installation directory."""
    override = os.environ.get("DOUYIN_MONITOR_HOME")
    if override:
        return Path(override).expanduser().resolve()
    local = os.environ.get("LOCALAPPDATA")
    if local:
        return Path(local) / "DouyinLiveMonitor"
    return Path.home() / ".local" / "share" / "DouyinLiveMonitor"


def parse_room_id(value: str) -> str:
    """Accept a web room ID or a supported Douyin URL, never arbitrary URLs."""
    if not isinstance(value, str):
        raise ValueError("请输入数字直播间 ID 或抖音直播间链接")
    value = value.strip()
    if re.fullmatch(r"[0-9]{1,25}", value):
        return value
    try:
        parsed = urlsplit(value)
        if parsed.scheme != "https" or parsed.username or parsed.password:
            raise ValueError
        if parsed.netloc.lower() == "live.douyin.com":
            match = re.fullmatch(r"/([0-9]{1,25})/?", parsed.path)
        elif parsed.netloc.lower() in {"www.douyin.com", "douyin.com"}:
            match = re.fullmatch(r"/follow/live/([0-9]{1,25})/?", parsed.path)
        else:
            match = None
        if match:
            return match.group(1)
    except ValueError:
        pass
    raise ValueError("仅支持数字 ID、https://live.douyin.com/数字 或抖音 /follow/live/数字 链接")


def _crypt_cookie(data: bytes, decrypt: bool = False) -> bytes:
    """Use the current Windows user's DPAPI key; never fall back to plaintext."""
    if os.name != "nt":
        raise ConfigError("保存 Cookie 需要 Windows 用户凭据保护；可留空 Cookie 运行")
    from ctypes import wintypes

    class DataBlob(ctypes.Structure):
        _fields_ = [("cbData", wintypes.DWORD), ("pbData", ctypes.POINTER(ctypes.c_ubyte))]

    crypt32 = ctypes.WinDLL("crypt32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    operation = crypt32.CryptUnprotectData if decrypt else crypt32.CryptProtectData
    operation.argtypes = [
        ctypes.POINTER(DataBlob), ctypes.c_void_p, ctypes.c_void_p,
        ctypes.c_void_p, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(DataBlob),
    ]
    operation.restype = wintypes.BOOL
    kernel32.LocalFree.argtypes = [ctypes.c_void_p]
    kernel32.LocalFree.restype = ctypes.c_void_p
    buffer = ctypes.create_string_buffer(data)
    source = DataBlob(len(data), ctypes.cast(buffer, ctypes.POINTER(ctypes.c_ubyte)))
    destination = DataBlob()
    if not operation(ctypes.byref(source), None, None, None, None, 1, ctypes.byref(destination)):
        action = "解密" if decrypt else "加密"
        raise ConfigError(f"Cookie {action}失败，请使用保存它的 Windows 用户，或重新设置 Cookie")
    try:
        return ctypes.string_at(destination.pbData, destination.cbData)
    finally:
        kernel32.LocalFree(ctypes.cast(destination.pbData, ctypes.c_void_p))


def _validated_config(value: object) -> dict:
    if not isinstance(value, dict):
        raise ConfigError("配置格式错误：应为 JSON 对象")
    result = dict(value)
    anchors = result.get("anchors", {})
    if not isinstance(anchors, dict):
        raise ConfigError("配置格式错误：主播列表应为对象")
    clean_anchors = {}
    for rid, name in anchors.items():
        try:
            clean_rid = parse_room_id(rid)
        except ValueError as exc:
            raise ConfigError("配置中的直播间 ID 无效") from exc
        if not isinstance(name, str):
            raise ConfigError("配置中的主播名称应为文本")
        clean_anchors[clean_rid] = name.strip() or clean_rid
    result["anchors"] = clean_anchors
    # Only explicit, user-confirmed marks are stored. Never infer browser
    # support from a failed network response or add a key to old configurations.
    if "pc_unsupported_rids" in result:
        marked = result["pc_unsupported_rids"]
        if not isinstance(marked, list) or any(
            not isinstance(rid, str) or not re.fullmatch(r"[0-9]{1,25}", rid)
            for rid in marked
        ):
            raise ConfigError("非 PC 支持标记配置无效，应为数字直播间 ID 列表")
        result["pc_unsupported_rids"] = list(dict.fromkeys(rid for rid in marked if rid in clean_anchors))
    def clean_cookie(cookie):
        if not isinstance(cookie, str) or "\r" in cookie or "\n" in cookie or len(cookie) > 65536:
            raise ConfigError("Cookie 格式无效，请粘贴完整的单行 Cookie")
        return cookie.strip()
    result["cookie"] = clean_cookie(result.get("cookie", ""))
    anchor_cookies = result.get("anchor_cookies", {})
    if not isinstance(anchor_cookies, dict):
        raise ConfigError("各直播间 Cookie 配置格式错误")
    clean_cookies = {}
    for rid, cookie in anchor_cookies.items():
        try:
            clean_rid = parse_room_id(rid)
        except ValueError as exc:
            raise ConfigError("Cookie 配置中的直播间 ID 无效") from exc
        cookie = clean_cookie(cookie)
        if cookie:
            clean_cookies[clean_rid] = cookie
    result["anchor_cookies"] = clean_cookies
    metadata = result.get("anchor_metadata", {})
    if not isinstance(metadata, dict):
        raise ConfigError("主播身份配置应为对象")
    clean_metadata = {}
    for rid, item in metadata.items():
        try:
            clean_rid = parse_room_id(rid)
        except ValueError as exc:
            raise ConfigError("主播身份配置中的直播间 ID 无效") from exc
        if not isinstance(item, dict):
            raise ConfigError("主播身份配置格式无效")
        anchor_id = item.get("anchor_id", "")
        if not isinstance(anchor_id, str) or not re.fullmatch(r"[0-9]{1,64}", anchor_id) or int(anchor_id) == 0:
            raise ConfigError("主播身份配置缺少真实主播 ID")
        clean_metadata[clean_rid] = dict(item)
    result["anchor_metadata"] = clean_metadata
    proxy = result.get("proxy_pool", default_proxy_pool())
    if (not isinstance(proxy, dict) or not isinstance(proxy.get("enabled", True), bool)
            or not isinstance(proxy.get("provider", "scdn"), str) or not isinstance(proxy.get("api_url", ""), str)):
        raise ConfigError("代理池配置格式无效")
    if proxy.get("provider", "scdn") != "scdn" or proxy.get("api_url", "") not in ("", default_proxy_pool()["api_url"]):
        raise ConfigError("代理池仅支持已配置的 scdn 公共接口")
    if "schema_version" in proxy and (type(proxy["schema_version"]) is not int or proxy["schema_version"] != 1):
        raise ConfigError("代理池配置版本无效")
    if proxy.get("protocol", "https") != "https":
        raise ConfigError("代理池来源协议固定为 HTTPS")
    # Pre-release reserved disabled fields were never a user setting. Only the
    # versioned form records the new Settings screen's explicit direct mode.
    enabled = proxy.get("enabled", True) if proxy.get("schema_version") == 1 else True
    source = proxy.get("source", "public")
    if source not in ("public", "imported"):
        raise ConfigError("代理来源须为公共代理或已导入代理")
    include_direct = proxy.get("include_direct", True)
    if type(include_direct) is not bool:
        raise ConfigError("直连出口选项须为布尔值")
    result["proxy_pool"] = {**default_proxy_pool(), "enabled": enabled, "source": source, "include_direct": include_direct}
    from proxy_import import ProxyImportError, validate_records
    try:
        result["imported_proxies"] = validate_records(result.get("imported_proxies", []))
    except ProxyImportError:
        raise ConfigError("已导入代理配置格式无效，请重新导入") from None
    from subscription_store import SubscriptionError, merged_candidates, validate_subscriptions
    try:
        result["proxy_subscriptions"] = validate_subscriptions(result.get("proxy_subscriptions", []))
        merged_candidates(result)
    except (SubscriptionError, ProxyImportError, ValueError, TypeError):
        raise ConfigError("代理订阅配置格式无效，请重新添加订阅") from None
    return result


def save_config(config: dict, data_dir: str | Path | None = None) -> None:
    """Replace settings atomically, preserving custom keys and encrypting Cookie."""
    clean = _validated_config(config)
    cookie = clean.pop("cookie")
    anchor_cookies = clean.pop("anchor_cookies")
    imported_proxies = clean.pop("imported_proxies")
    subscriptions = clean.pop("proxy_subscriptions")
    clean.pop("cookie_encrypted", None)
    clean.pop("anchor_cookies_encrypted", None)
    clean.pop("imported_proxies_encrypted", None)
    clean.pop("proxy_subscriptions_encrypted", None)
    clean["schema_version"] = 2
    if cookie:
        clean["cookie_encrypted"] = base64.b64encode(_crypt_cookie(cookie.encode("utf-8"))).decode("ascii")
    if anchor_cookies:
        encoded_cookies = json.dumps(anchor_cookies, ensure_ascii=False).encode("utf-8")
        clean["anchor_cookies_encrypted"] = base64.b64encode(_crypt_cookie(encoded_cookies)).decode("ascii")
    if imported_proxies:
        try:
            encoded_proxies = json.dumps(imported_proxies, ensure_ascii=False).encode("utf-8")
            clean["imported_proxies_encrypted"] = base64.b64encode(_crypt_cookie(encoded_proxies)).decode("ascii")
        except ConfigError:
            raise ConfigError("代理凭据加密失败；需要当前 Windows 用户的 DPAPI 保护，未保存明文") from None
    if subscriptions:
        try:
            encoded = json.dumps(subscriptions, ensure_ascii=False).encode("utf-8")
            clean["proxy_subscriptions_encrypted"] = base64.b64encode(_crypt_cookie(encoded)).decode("ascii")
        except ConfigError:
            raise ConfigError("订阅信息加密失败，未保存明文") from None
    target_dir = Path(data_dir) if data_dir is not None else app_data_dir()
    temporary = None
    try:
        content = json.dumps(clean, ensure_ascii=False, indent=2) + "\n"
        target_dir.mkdir(parents=True, exist_ok=True)
        with tempfile.NamedTemporaryFile(
            mode="w", encoding="utf-8", prefix=".settings-", suffix=".tmp", dir=target_dir, delete=False
        ) as stream:
            temporary = Path(stream.name)
            stream.write(content)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, target_dir / "config.json")
    except (OSError, TypeError, ValueError) as exc:
        raise ConfigError("无法保存配置，请检查用户数据目录的权限和剩余磁盘空间") from exc
    finally:
        if temporary is not None:
            try:
                temporary.unlink(missing_ok=True)
            except OSError:
                pass


def load_config(
    data_dir: str | Path | None = None, legacy_path: str | Path | None = None
) -> dict:
    """Read settings; import only a known adjacent legacy config on the first run.

    The original legacy file is preserved, because it belongs to the old app.
    Imported credentials are protected in the new copy before this returns.
    """
    target_dir = Path(data_dir) if data_dir is not None else app_data_dir()
    target = target_dir / "config.json"
    if legacy_path is None:
        # Moving source into python/collector must not change the known legacy
        # config location used by a source checkout. Frozen builds stay adjacent.
        source_dir = Path(sys.executable).parent if getattr(sys, "frozen", False) else Path(__file__).resolve().parents[2]
        legacy_path = source_dir / "config.json"
    legacy = Path(legacy_path)
    source = target if target.exists() else legacy
    if not source.exists():
        return {"anchors": {}, "cookie": "", "anchor_cookies": {}, "anchor_metadata": {}, "proxy_pool": default_proxy_pool(), "imported_proxies": [], "proxy_subscriptions": []}
    try:
        raw = json.loads(source.read_text(encoding="utf-8-sig"))
    except (OSError, UnicodeError, ValueError) as exc:
        raise ConfigError("无法读取配置；原文件已保留，请检查 config.json 或移走损坏的文件后重试") from exc
    config = _validated_config(raw)
    encrypted = config.pop("cookie_encrypted", None)
    encrypted_anchors = config.pop("anchor_cookies_encrypted", None)
    encrypted_proxies = config.pop("imported_proxies_encrypted", None)
    encrypted_subscriptions = config.pop("proxy_subscriptions_encrypted", None)
    if encrypted:
        try:
            config["cookie"] = _crypt_cookie(base64.b64decode(encrypted, validate=True), decrypt=True).decode("utf-8")
        except ConfigError:
            raise
        except (ValueError, TypeError, UnicodeError) as exc:
            raise ConfigError("已保存的 Cookie 无法读取，请重新设置 Cookie") from exc
        config = _validated_config(config)
    if encrypted_anchors:
        try:
            decrypted = _crypt_cookie(base64.b64decode(encrypted_anchors, validate=True), decrypt=True)
            config["anchor_cookies"] = json.loads(decrypted.decode("utf-8"))
        except ConfigError:
            raise
        except (ValueError, TypeError, UnicodeError) as exc:
            raise ConfigError("已保存的直播间 Cookie 无法读取，请重新设置 Cookie") from exc
        config = _validated_config(config)
    if encrypted_proxies:
        try:
            decrypted = _crypt_cookie(base64.b64decode(encrypted_proxies, validate=True), decrypt=True)
            config["imported_proxies"] = json.loads(decrypted.decode("utf-8"))
            config = _validated_config(config)
        except (ConfigError, ValueError, TypeError, UnicodeError):
            raise ConfigError("已保存的代理凭据无法读取，请使用保存它的 Windows 用户或重新导入") from None
    if encrypted_subscriptions:
        try:
            decrypted = _crypt_cookie(base64.b64decode(encrypted_subscriptions, validate=True), decrypt=True)
            config["proxy_subscriptions"] = json.loads(decrypted.decode("utf-8"))
            config = _validated_config(config)
        except (ConfigError, ValueError, TypeError, UnicodeError):
            raise ConfigError("已保存的订阅无法读取，请使用保存它的 Windows 用户或重新添加") from None
    if source.resolve() != target.resolve() or any(key in raw for key in ("cookie", "anchor_cookies", "imported_proxies", "proxy_subscriptions")):
        save_config(config, target_dir)
    return config
