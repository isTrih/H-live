"""Room-level API refusal must not blacklist a healthy proxy lease."""
from pathlib import Path
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from liveMan import LiveMonitorError, LiveRoomUnavailableError
from monitor import MonitorManager
from test_proxy_integration import LeasePool, until


class RoomRefusalLeaseTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)

    def manager(self, pool, lookup, factory, events):
        manager = MonitorManager(data_dir=self.directory.name, proxy_pool=pool,
            room_info_fetcher=lookup, fetcher_factory=factory, on_event=events.append,
            retry_seconds=.005, offline_seconds=.005, connect_interval_seconds=0)
        self.addCleanup(lambda: manager.shutdown(3))
        return manager

    def test_initial_and_persistent_room_refusal_keep_same_proxy_without_fabricating_offline(self):
        for previously_live in (False, True):
            with self.subTest(previously_live=previously_live):
                pool, routes, events = LeasePool(), [], []
                entered, resume = threading.Event(), threading.Event()
                self.addCleanup(resume.set)

                def lookup(rid, cookie, proxy):
                    routes.append(proxy)
                    if previously_live and len(routes) == 1:
                        return {'is_live': True, 'anchor_id': '111', 'room_id': '222'}
                    if len(routes) >= 5:
                        entered.set()
                        resume.wait(2)
                    raise LiveRoomUnavailableError()

                class RefusingFetcher:
                    def __init__(self, rid, on_status, **kwargs):
                        self.on_status = on_status
                    def start(self):
                        self.on_status('connected', 'synthetic fixture')
                        raise LiveRoomUnavailableError()
                    def stop(self):
                        pass

                manager = self.manager(pool, lookup, RefusingFetcher, events)
                manager.start('1', 'fixture')
                self.assertTrue(entered.wait(2))
                self.assertEqual(len(pool.calls), 1)
                self.assertTrue(all(route is routes[0] for route in routes))
                self.assertEqual(pool.invalidations, [])
                states = [event.get('state') for event in events]
                self.assertNotIn('offline', states)
                self.assertIn('retrying', states)
                self.assertEqual('confirming_offline' in states, previously_live)
                self.assertIs(manager.snapshot()['1']['is_live'], True if previously_live else None)
                manager.stop('1')
                resume.set()
                until(manager.can_configure_proxy)
                self.assertEqual(len(pool.releases), 1)
                self.assertTrue(manager.shutdown(3))

    def test_transport_error_still_invalidates_before_next_room_query(self):
        pool, routes, events = LeasePool(), [], []
        entered, resume = threading.Event(), threading.Event()
        self.addCleanup(resume.set)

        def lookup(rid, cookie, proxy):
            routes.append(proxy)
            if len(routes) == 1:
                raise LiveMonitorError('合成网络连接失败')
            entered.set()
            resume.wait(2)
            return {'is_live': False}

        manager = self.manager(pool, lookup, lambda *args, **kwargs: None, events)
        manager.start('1', 'fixture')
        self.assertTrue(entered.wait(2))
        self.assertEqual(len(pool.invalidations), 1)
        self.assertIs(pool.invalidations[0][1], routes[0])
        self.assertIsNot(routes[0], routes[1])
        manager.stop('1')
        resume.set()
        until(manager.can_configure_proxy)


if __name__ == '__main__':
    unittest.main()
