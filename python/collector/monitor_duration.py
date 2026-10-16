"""Cumulative observed time, keyed by real broadcaster and broadcast identity."""
from dataclasses import dataclass, field
import threading
from time import monotonic, time


@dataclass(eq=False)
class _Duration:
    key: tuple
    seconds: float
    persisted: float
    members: set = field(default_factory=set)
    running: set = field(default_factory=set)
    since: float | None = None
    started: bool = False


class DurationLedger:
    """One clock per session, including when multiple web entries overlap.

    Reads/writes never hold the state lock. Checkpoints contain elapsed time,
    not an open interval, so a later process cannot count its downtime.
    """

    def __init__(self, archive, on_error=None, checkpoint_seconds=5, clock=None, wall_clock=None):
        self.archive = archive
        self._clock, self._wall = clock or monotonic, wall_clock or time
        self._on_error = on_error
        self._interval = checkpoint_seconds
        self._lock, self._io_lock = threading.RLock(), threading.Lock()
        self._entries, self._bindings = {}, {}
        self._wake, self._closing = threading.Event(), threading.Event()
        self._last_flush_ok = True
        self._next_checkpoint = self._clock() + self._interval
        self._thread = threading.Thread(target=self._run, name="monitor-duration-checkpoints", daemon=True)
        self._thread.start()

    def bind(self, token, anchor_id, session_id):
        key = (anchor_id, session_id)
        with self._io_lock:
            with self._lock:
                if self._closing.is_set():
                    raise RuntimeError("监控时长已关闭")
                existing = self._bindings.get(token)
                if existing is not None and existing.key == key:
                    return
                entry = self._entries.get(key)
            if entry is None:
                saved = self.archive.load_monitor_duration(*key)
                entry = _Duration(key, saved, saved, started=saved > 0)
            with self._lock:
                if existing is not None:
                    self._pause_locked(token, existing)
                    existing.members.discard(token)
                self._entries.setdefault(key, entry)
                entry = self._entries[key]
                entry.members.add(token)
                self._bindings[token] = entry

    def _seconds(self, entry):
        return entry.seconds + (max(0, self._clock() - entry.since) if entry.since is not None else 0)

    def resume(self, token, initial_seconds=0):
        with self._lock:
            entry = self._bindings.get(token)
            if entry is None or self._closing.is_set() or token in entry.running:
                return
            if not entry.running:
                if not entry.started:
                    entry.seconds = max(entry.seconds, max(0, initial_seconds))
                entry.since = self._clock()
            entry.started = True
            entry.running.add(token)

    def _pause_locked(self, token, entry):
        entry.running.discard(token)
        if not entry.running and entry.since is not None:
            entry.seconds = self._seconds(entry)
            entry.since = None

    def pause(self, token):
        with self._lock:
            entry = self._bindings.get(token)
            if entry is not None:
                self._pause_locked(token, entry)
        self._wake.set()

    def snapshot(self, token):
        with self._lock:
            entry = self._bindings.get(token)
            if entry is None:
                return None
            return {"duration_seconds": self._seconds(entry),
                    "duration_running": token in entry.running,
                    "duration_updated_at": self._wall()}

    def unbind(self, token):
        self.pause(token)
        with self._lock:
            entry = self._bindings.pop(token, None)
            if entry is not None:
                entry.members.discard(token)
        self._wake.set()

    def flush(self, include_running=True):
        try:
            with self._io_lock:
                with self._lock:
                    pending = [(entry, self._seconds(entry)) for entry in self._entries.values()
                               if include_running or entry.since is None]
                for entry, seconds in pending:
                    if seconds > entry.persisted:
                        self.archive.save_monitor_duration(*entry.key, seconds)
                        with self._lock:
                            entry.persisted = max(entry.persisted, seconds)
                    with self._lock:
                        if not entry.members and entry.since is None and entry.seconds <= entry.persisted:
                            self._entries.pop(entry.key, None)
            self._last_flush_ok = True
            return True
        except Exception:
            self._last_flush_ok = False
            if self._on_error is not None:
                self._on_error()
            return False

    def _run(self):
        while True:
            self._wake.wait(max(0, self._next_checkpoint - self._clock()))
            self._wake.clear()
            final_flush = self._closing.is_set()
            periodic = self._clock() >= self._next_checkpoint
            # A disconnect saves its paused entry promptly without causing every
            # healthy session to commit again. Wakes do not postpone the deadline.
            self.flush(include_running=final_flush or periodic)
            if periodic:
                self._next_checkpoint = self._clock() + self._interval
            if final_flush:
                return

    def close(self, timeout=10):
        with self._lock:
            for token, entry in self._bindings.items():
                self._pause_locked(token, entry)
            self._closing.set()
        self._wake.set()
        self._thread.join(max(0, timeout))
        return not self._thread.is_alive() and self._last_flush_ok
