"""Cancellable admission limits for one network exit, independent of sockets."""

import math
import threading
import time


class RouteControl:
    def __init__(self, connect_limit=2, request_limit=2, request_interval_seconds=0.5):
        if (connect_limit < 1 or request_limit < 1 or
                not math.isfinite(request_interval_seconds) or request_interval_seconds < 0):
            raise ValueError("出口限速参数无效")
        self._condition = threading.Condition()
        self._limits = {"connecting": int(connect_limit), "request_inflight": int(request_limit)}
        self._counts = dict.fromkeys(self._limits, 0)
        self._interval = float(request_interval_seconds)
        self._next_request = 0.0
        self._cooldown_until = 0.0
        self._closed = False

    def _acquire(self, kind, stop_event):
        with self._condition:
            while not self._closed and not (stop_event is not None and stop_event.is_set()):
                now = time.monotonic()
                allowed_at = self._cooldown_until
                if kind == "request_inflight":
                    allowed_at = max(allowed_at, self._next_request)
                if self._counts[kind] < self._limits[kind] and now >= allowed_at:
                    self._counts[kind] += 1
                    if kind == "request_inflight":
                        # Reserve only an admitted start, never a cancelled future waiter.
                        self._next_request = now + self._interval
                    released = False

                    def release():
                        nonlocal released
                        with self._condition:
                            if not released:
                                released = True
                                self._counts[kind] -= 1
                                self._condition.notify_all()

                    return release
                self._condition.wait(min(0.1, max(0.001, allowed_at - now))
                                     if now < allowed_at else 0.1)
        return None

    def acquire_connect(self, stop_event=None):
        return self._acquire("connecting", stop_event)

    def acquire_request(self, stop_event=None):
        return self._acquire("request_inflight", stop_event)

    def cooldown(self, seconds):
        if not math.isfinite(seconds) or seconds < 0:
            raise ValueError("出口冷却时间无效")
        with self._condition:
            self._cooldown_until = max(self._cooldown_until, time.monotonic() + seconds)
            self._condition.notify_all()

    def snapshot(self):
        with self._condition:
            return {**self._counts, "cooldown_seconds": max(0.0, self._cooldown_until - time.monotonic()),
                    "closed": self._closed}

    def close(self):
        """Wake pending admissions; existing holders still release their permits."""
        with self._condition:
            self._closed = True
            self._condition.notify_all()
