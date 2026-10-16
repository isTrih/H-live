"""Compatibility adapter: all archive iteration and file writing run in Rust."""
import json
from datetime import datetime
from pathlib import Path
import re
import threading

from qingkan_native import CancelToken, ExportCancelled, ExportDataError
from qingkan_native import export_month as _native_export

HEADERS = ("主播", "直播间场次", "发消息人ID", "发消息人名字", "消息类型", "消息内容", "发消息时间")
MESSAGE_TYPE_LABELS = {"chat": "弹幕", "gift": "礼物", "like": "点赞", "member": "进场", "social": "关注/分享", "fansclub": "粉丝团", "emoji": "表情弹幕", "activity": "红包/福袋"}

def normalize_month(month):
    if not isinstance(month, str) or not re.fullmatch(r"[0-9]{4}-[0-9]{2}", month.strip()):
        raise ValueError("月份须为 YYYY-MM，例如 2026-10。")
    month = month.strip()
    datetime(int(month[:4]), int(month[5:]), 1)
    return month

def _values(values, label):
    if values is None or isinstance(values, (str, bytes)):
        raise ValueError(f"{label}须为列表。")
    result = list(dict.fromkeys(str(value).strip() for value in values))
    if any(not value for value in result):
        raise ValueError(f"{label}不能包含空值。")
    return result

def export_month(data_dir, anchor_ids, month, output_path, format=None, types=None, cancel_event=None):
    month = normalize_month(month)
    ids = _values(anchor_ids, "主播")
    selected_types = list(MESSAGE_TYPE_LABELS) if types is None else _values(types, "消息类型")
    destination = Path(output_path).expanduser().absolute()
    kind = str(format or destination.suffix.lstrip(".")).lower().lstrip(".")
    from app_storage import app_data_dir
    root = Path(data_dir) if data_dir is not None else app_data_dir()
    token = cancel_event if isinstance(cancel_event, CancelToken) else CancelToken()
    done = threading.Event()
    watcher = None
    if cancel_event is not None and cancel_event is not token:
        if cancel_event.is_set():
            token.set()
        else:
            # Legacy diagnostic callers can still provide threading.Event.
            # Product jobs should own CancelToken directly: no watcher needed.
            def watch():
                while not done.wait(.02):
                    if cancel_event.is_set():
                        token.set()
                        return
            watcher = threading.Thread(target=watch, name="export-cancel-adapter", daemon=True)
            watcher.start()
    try:
        return json.loads(_native_export(str(root), ids, month, str(destination), kind, selected_types, token))
    finally:
        done.set()
        if watcher is not None:
            watcher.join(.1)
