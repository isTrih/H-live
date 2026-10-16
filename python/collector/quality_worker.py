"""One cancellable worker; analysis is durable and never runs in a WS callback."""
import threading
from qingkan_native import CancelToken, export_quality as _export_quality
import json


class QualityWorker:
    def __init__(self, archive):
        self._archive = archive
        self._cancel = CancelToken()
        self._stop = threading.Event()
        self._error = False
        self._thread = None
        if callable(getattr(archive, "quality", None)):
            self._thread = threading.Thread(target=self._run, name="session-quality-analysis", daemon=True)
            self._thread.start()

    def _run(self):
        while not self._stop.is_set():
            delay = .15
            try:
                result = self._archive.quality("step", {"limit": 128}, self._cancel)
                self._error = False
                if result.get("processed", 0):
                    delay = .01
            except Exception:
                # A busy DB or cancelled transaction is retried from its last
                # committed cursor. No raw exception/data is published to UI.
                self._error = not self._stop.is_set()
                delay = 1
            self._stop.wait(delay)

    def status(self):
        return {"available": self._thread is not None, "retrying": self._error}

    def close(self, timeout=5):
        self._stop.set()
        self._cancel.set()
        if self._thread is not None:
            self._thread.join(max(0, timeout))
        return self._thread is None or not self._thread.is_alive()


def export_session_quality(data_dir, anchor_ids, month, output_path, *, format="xlsx", types=None, cancel_event=None):
    if len(anchor_ids) != 1:
        raise ValueError("请从一个主播的详情导出场次分析")
    token = cancel_event if cancel_event is not None else CancelToken()
    return json.loads(_export_quality(str(data_dir), anchor_ids[0], month, str(output_path), format, token))
