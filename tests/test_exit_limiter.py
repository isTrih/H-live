import sys
from pathlib import Path
import threading
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
from route_control import RouteControl


class ExitLimiterTests(unittest.TestCase):
    def test_parallel_connect_and_request_limits_and_idempotent_release(self):
        control = RouteControl(request_interval_seconds=0)
        connect = [control.acquire_connect() for _ in range(2)]
        request = [control.acquire_request() for _ in range(2)]
        stop, finished = threading.Event(), []
        threads = [threading.Thread(target=lambda method=method: finished.append(method(stop)))
                   for method in (control.acquire_connect, control.acquire_request)]
        for thread in threads:
            thread.start()
        time.sleep(0.03)
        self.assertEqual(finished, [])
        self.assertEqual(control.snapshot()["connecting"], 2)
        self.assertEqual(control.snapshot()["request_inflight"], 2)
        stop.set()
        for thread in threads:
            thread.join(0.2)
            self.assertFalse(thread.is_alive())
        self.assertEqual(finished, [None, None])
        for release in connect + request:
            release()
            release()
        self.assertEqual(control.snapshot()["connecting"], 0)
        self.assertEqual(control.snapshot()["request_inflight"], 0)

    def test_cancelled_waiters_do_not_reserve_future_request_slots(self):
        control = RouteControl(request_limit=1, request_interval_seconds=0.08)
        first = control.acquire_request()
        # Keep capacity occupied until cancellation. Starting 20 OS threads
        # can exceed 80 ms on a busy machine; elapsed time must not admit them.
        stop = threading.Event()
        results = []
        threads = [threading.Thread(target=lambda: results.append(control.acquire_request(stop))) for _ in range(20)]
        for thread in threads:
            thread.start()
        stop.set()
        for thread in threads:
            thread.join(2)
            self.assertFalse(thread.is_alive())
        self.assertTrue(all(result is None for result in results))
        first()
        begin = time.monotonic()
        release = control.acquire_request()
        # Reserving 20 future slots would take 1.6 s. Allow scheduling jitter.
        self.assertLess(time.monotonic() - begin, 0.5)
        release()

    def test_request_start_spacing_and_inflight_limit(self):
        control = RouteControl(request_interval_seconds=0.03)
        starts = []
        lock = threading.Lock()
        def work():
            release = control.acquire_request()
            with lock:
                starts.append(time.monotonic())
            time.sleep(0.02)
            release()
        threads = [threading.Thread(target=work) for _ in range(6)]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join(1)
            self.assertFalse(thread.is_alive())
        self.assertTrue(all(b - a >= 0.027 for a, b in zip(starts, starts[1:])))
        self.assertEqual(control.snapshot()["request_inflight"], 0)

    def test_cooldown_blocks_new_admissions_not_existing_holders_or_other_exit(self):
        first, second = RouteControl(request_interval_seconds=0), RouteControl(request_interval_seconds=0)
        release = first.acquire_connect()
        first.cooldown(0.1)
        self.assertEqual(first.snapshot()["connecting"], 1)
        second.acquire_connect()()
        begin = time.monotonic()
        first.acquire_request()()
        self.assertGreaterEqual(time.monotonic() - begin, 0.09)
        release()

    def test_close_wakes_waiters_without_losing_active_permit_accounting(self):
        control = RouteControl()
        release = control.acquire_connect()
        control.cooldown(10)
        result = []
        thread = threading.Thread(target=lambda: result.append(control.acquire_request()))
        thread.start()
        control.close()
        thread.join(0.2)
        self.assertEqual(result, [None])
        self.assertIsNone(control.acquire_connect())
        release()
        self.assertEqual(control.snapshot()["connecting"], 0)


if __name__ == "__main__":
    unittest.main()
