"""Synthetic imports only: no public network and no user credentials."""
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import MagicMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "python" / "collector"))
from app_storage import ConfigError, default_proxy_pool, load_config, save_config
from backend_service import BackendService, ServiceError
from proxy_import import ProxyImportError, make_record, parse_proxy_text
from proxy_pool import ProxyPool, validate_proxy


def fixture(ip='8.8.8.8', protocol='socks5'):
    return {'ip': ip, 'port': 1080, 'username': 'fixture-user', 'password': 'fixture:pass/@',
            'protocol': protocol, 'city': '测试城市', 'country_code': 'CN', 'expires_at': None,
            'status': 'available', 'proxy_uid': 'ignored-vendor-identifier'}


def until(predicate, seconds=3):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.005)
    raise AssertionError('condition not reached')


class ParserTests(unittest.TestCase):
    def test_json_vendor_schema_is_write_only_and_does_not_trust_status(self):
        records, preview = parse_proxy_text(json.dumps([fixture()]))
        self.assertEqual(records[0]['protocol'], 'socks5h')
        self.assertEqual(records[0]['password'], 'fixture:pass/@')
        self.assertEqual(preview['items'][0]['status'], 'unchecked')
        self.assertTrue(preview['items'][0]['auth_present'])
        for secret in ('fixture-user', 'fixture:pass/@', 'ignored-vendor-identifier'):
            self.assertNotIn(secret, json.dumps(preview))

    def test_url_and_ipv6_and_percent_encoded_credentials(self):
        records, preview = parse_proxy_text('http://user%40domain:pass%3A%40%2F@8.8.8.8:80\n[2606:4700:4700::1111]:1080:other:password', 'socks5')
        self.assertEqual(preview['valid'], 2)
        self.assertEqual(records[0]['username'], 'user@domain')
        self.assertEqual(records[0]['password'], 'pass:@/')
        self.assertEqual(records[0]['protocol'], 'http')
        self.assertEqual(records[1]['protocol'], 'socks5h')

    def test_header_table_maps_auth_explicitly_and_ignores_vendor_status(self):
        text = 'IP\t城市\t端口\t账号\t密码\t更新时间\t到期\t状态\n8.8.8.8\t北京\t8080\trow-user\trow-pass\t2026-10-14 10:00:00\t2030-10-14 10:00:00\t正常'
        records, preview = parse_proxy_text(text, 'socks5')
        self.assertEqual(preview['valid'], 1)
        self.assertEqual(records[0]['username'], 'row-user')
        self.assertEqual(records[0]['password'], 'row-pass')
        self.assertEqual(records[0]['expires_at'], '2030-10-14T10:00:00+08:00')
        self.assertEqual(preview['items'][0]['status'], 'unchecked')

    def test_headerless_table_requires_recognized_columns_and_valid_date(self):
        records, preview = parse_proxy_text('8.8.8.8 北京 80 account password 2026-10-14 10:00:00 2030-10-14 10:00:00 正常')
        self.assertEqual(preview['valid'], 1)
        self.assertEqual(records[0]['username'], 'account')
        _, rejected = parse_proxy_text('8.8.8.8 北京 80 2026-10-14 2030-10-14')
        self.assertEqual(rejected['invalid'], 1)
        self.assertNotIn('北京', json.dumps(rejected))

    def test_whitespace_table_dates_merge_only_for_explicit_date_columns(self):
        text = 'IP 城市 端口 账号 密码 更新时间 到期时间 状态\n8.8.8.8 北京 80 user pass 2026-10-14 10:00:00 2030-10-14 11:00:00 正常'
        records, preview = parse_proxy_text(text)
        self.assertEqual(preview['valid'], 1)
        self.assertEqual(records[0]['username'], 'user')
        self.assertEqual(records[0]['expires_at'], '2030-10-14T11:00:00+08:00')

    def test_protocol_credential_constraints_match_both_transports(self):
        for record in ({**fixture(protocol='http'), 'username': 'user:name'},
                       {**fixture(protocol='http'), 'username': '账号'},
                       {**fixture(), 'password': '字' * 86},
                       {**fixture(), 'ip': '2606:4700:4700::1111%scope'}):
            with self.subTest(record=record), self.assertRaises(ProxyImportError):
                make_record(record)

    def test_changed_existing_endpoint_is_updated_and_repeated_input_is_first_wins(self):
        records, _ = parse_proxy_text('8.8.8.8:80:first:secret')
        new, preview = parse_proxy_text('8.8.8.8:80:second:newsecret\n8.8.8.8:80:third:ignored\n1.1.1.1:80\n1.1.1.1:80', existing=records)
        self.assertEqual((preview['total'], preview['valid'], preview['added'], preview['updated'], preview['duplicates']), (4, 2, 1, 1, 2))
        self.assertEqual(new[0]['username'], 'second')
        self.assertEqual(new[1]['endpoint'], '1.1.1.1:80')
        self.assertEqual(records[0]['username'], 'first')
        unchanged, summary = parse_proxy_text('8.8.8.8:80:first:secret', existing=records)
        self.assertEqual(unchanged, [])
        self.assertEqual((summary['valid'], summary['updated'], summary['duplicates']), (0, 0, 1))

    def test_invalid_credentials_paths_private_addresses_do_not_echo_input(self):
        text = '\n'.join(['http://user:very-private@127.0.0.1:80',
                          'http://user:very-private@8.8.8.8:80/path', '8.8.8.8:70000:user:very-private'])
        _, preview = parse_proxy_text(text)
        self.assertEqual(preview['invalid'], 3)
        self.assertNotIn('very-private', json.dumps(preview))
        with self.assertRaises(ProxyImportError):
            parse_proxy_text('[{"password":"private-unterminated')


class ImportedPoolTests(unittest.TestCase):
    def test_imported_pool_is_lazy_and_never_calls_public_source(self):
        record = make_record(fixture())
        with patch('proxy_pool.fetch_candidates', side_effect=AssertionError('public network forbidden')):
            pool = ProxyPool(imported_candidates=[record], validator=lambda endpoint: {'ok': True, 'status_code': 200})
            self.addCleanup(pool.shutdown)
            self.assertEqual(pool.snapshot()['fetch_count'], 0)
            self.assertIsNone(pool._thread)
            self.assertTrue(pool.refresh())
            until(lambda: not pool.snapshot()['refresh']['active'])
            self.assertEqual(pool.snapshot()['refresh']['target'], 1)
            self.assertEqual(pool.candidate_status(record['endpoint']), 'available')
            lease = pool.acquire('111', threading.Event())
            self.assertEqual(lease.proxy_url, 'socks5h://fixture-user:fixture%3Apass%2F%40@8.8.8.8:1080')
            self.assertNotIn('fixture-user', repr(lease))
            self.assertNotIn('fixture', json.dumps(pool.snapshot()))
            self.assertEqual(pool.candidate_status(record['endpoint']), 'assigned')

    def test_authenticated_health_uses_exact_url_and_never_disables_tls(self):
        session = MagicMock()
        session.__enter__.return_value = session
        session.get.return_value.__enter__.return_value.status_code = 200
        url = 'socks5h://user:encoded%40pass@8.8.8.8:1080'
        with patch('proxy_pool.requests.Session', return_value=session):
            self.assertTrue(validate_proxy('8.8.8.8:1080', proxy_url=url)['ok'])
        self.assertFalse(session.trust_env)
        kwargs = session.get.call_args.kwargs
        self.assertEqual(kwargs['proxies'], {'http': url, 'https': url})
        self.assertTrue(kwargs['verify'])

    def test_imported_failures_do_not_leak_authenticated_exception(self):
        def bad(endpoint):
            raise RuntimeError('http://fixture-user:fixture:pass/@8.8.8.8:1080')
        pool = ProxyPool(imported_candidates=[make_record(fixture())], validator=bad)
        self.addCleanup(pool.shutdown)
        pool.refresh()
        until(lambda: pool.snapshot()['checks_total'] == 1)
        self.assertEqual(pool.candidate_status('8.8.8.8:1080'), 'failed')
        self.assertNotIn('fixture', json.dumps(pool.snapshot()))

    def test_expired_import_is_not_probed(self):
        record = fixture()
        record['expires_at'] = '2000-01-01T00:00:00+08:00'
        validator = MagicMock()
        pool = ProxyPool(imported_candidates=[make_record(record)], validator=validator)
        self.addCleanup(pool.shutdown)
        self.assertEqual(pool.candidate_status('8.8.8.8:1080'), 'expired')
        self.assertEqual(pool._fetch_imported(20), [])
        validator.assert_not_called()


class FakeManager:
    def __init__(self, proxy_pool=None, **kwargs):
        self.pool, self.active = proxy_pool, False
        self.archive = MagicMock()
        self.archive.list_anchors.return_value = []
    def can_configure_proxy(self):
        return not self.active
    def configure_proxy(self, pool):
        if self.pool is not None:
            self.pool.shutdown(0)
        self.pool = pool
    def snapshot(self):
        return {}
    def shutdown(self, timeout=0):
        return self.pool is None or self.pool.shutdown(timeout)


class ImportedBackendStorageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        network_guard = patch('proxy_pool.requests.Session', side_effect=AssertionError('network forbidden'))
        blocked_session = network_guard.start()
        self.addCleanup(network_guard.stop)
        self.addCleanup(blocked_session.assert_not_called)

    @unittest.skipUnless(os.name == 'nt', 'DPAPI requires Windows')
    def test_dpapi_roundtrip_hides_both_account_and_password_and_failure_is_atomic(self):
        record = make_record(fixture())
        save_config({'imported_proxies': [record]}, self.directory)
        before = (self.directory / 'config.json').read_bytes()
        self.assertNotIn(b'fixture-user', before)
        self.assertNotIn(b'fixture:pass', before)
        self.assertNotIn(b'8.8.8.8', before)
        self.assertIn(b'imported_proxies_encrypted', before)
        loaded = load_config(self.directory, self.directory / 'absent.json')
        self.assertEqual(loaded['imported_proxies'], [record])
        with patch('app_storage._crypt_cookie', side_effect=ConfigError('synthetic-private-error')):
            with self.assertRaises(ConfigError) as caught:
                save_config({'imported_proxies': [make_record(fixture('1.1.1.1'))]}, self.directory)
        self.assertNotIn('synthetic-private-error', str(caught.exception))
        self.assertEqual((self.directory / 'config.json').read_bytes(), before)

    def service(self):
        service = BackendService(self.directory, manager_factory=FakeManager)
        self.addCleanup(service.shutdown)
        return service

    def test_preview_import_list_and_reload_never_return_secrets_or_start_checks(self):
        service = self.service()
        text = json.dumps([fixture(), fixture()])
        before = service.dispatch('proxy_status')['fetch_count']
        preview = service.dispatch('preview_proxies', {'text': text})
        self.assertEqual((preview['valid'], preview['duplicates']), (1, 1))
        self.assertEqual(service.dispatch('list_imported_proxies')['total'], 0)
        imported = service.dispatch('import_proxies', {'text': text})
        self.assertEqual(imported['added'], 1)
        self.assertEqual(service.dispatch('proxy_status')['fetch_count'], before)
        for method in ('init', 'poll', 'list_imported_proxies', 'proxy_status'):
            serialized = json.dumps(service.dispatch(method))
            self.assertNotIn('fixture-user', serialized)
            self.assertNotIn('fixture:pass', serialized)
        self.assertEqual(service.dispatch('init')['settings']['proxy_source'], 'public')
        item_id = imported['items'][0]['id']
        self.assertEqual(service.dispatch('remove_imported_proxies', {'ids': [item_id]})['removed'], 1)
        self.assertEqual(service.dispatch('list_imported_proxies')['total'], 0)

    def test_direct_mode_can_check_imports_and_enable_reuses_prepared_pool(self):
        service = self.service()
        service.dispatch('import_proxies', {'text': json.dumps([fixture()])})
        service.dispatch('configure_proxy', {'enabled': False, 'source': 'imported'})
        self.assertIsNone(service.manager.pool)
        with patch('proxy_pool.validate_proxy', return_value={'ok': True, 'status_code': 200}):
            service.dispatch('refresh_proxies')
            pool = service._prepared_proxy_pool
            until(lambda: pool.snapshot()['available'] == 1)
        self.assertIsNone(service.manager.pool)
        self.assertEqual(service.dispatch('list_imported_proxies')['items'][0]['status'], 'available')
        service.dispatch('configure_proxy', {'enabled': True})
        self.assertIs(service.manager.pool, pool)
        self.assertIsNone(service._prepared_proxy_pool)

    def test_reimport_changed_credentials_rechecks_only_updated_proxy_through_rpc(self):
        service = self.service()
        original = [fixture(), fixture('1.1.1.1')]
        service.dispatch('import_proxies', {'text': json.dumps(original)})
        service.dispatch('configure_proxy', {'enabled': True, 'source': 'imported'})
        with patch('proxy_pool.validate_proxy', return_value={'ok': False, 'status_code': 407}):
            service.dispatch('refresh_proxies')
            until(lambda: service.dispatch('proxy_status')['checks_total'] == 2)
        replacement = {**original[0], 'username': 'updated-user', 'password': 'updated-password'}
        params = {'text': json.dumps([replacement])}
        preview = service.dispatch('preview_proxies', params)
        self.assertEqual(preview['valid'], 1, 'Changed credentials must be an actionable import')
        self.assertEqual((preview['valid'], preview['added'], preview['updated'], preview['duplicates']), (1, 0, 1, 0))
        self.assertEqual([item['status'] for item in service.dispatch('list_imported_proxies')['items']], ['failed', 'failed'])
        committed = service.dispatch('import_proxies', params)
        self.assertEqual((committed['added'], committed['updated']), (0, 1))
        items = service.dispatch('list_imported_proxies')['items']
        self.assertEqual([item['endpoint'] for item in items], ['8.8.8.8:1080', '1.1.1.1:1080'])
        self.assertEqual([item['status'] for item in items], ['unchecked', 'failed'])
        with patch('proxy_pool.validate_proxy', return_value={'ok': True, 'status_code': 200}) as validator:
            service.dispatch('refresh_proxies')
            until(lambda: service.dispatch('proxy_status')['checks_total'] == 1)
            self.assertEqual(validator.call_count, 1)
            self.assertEqual(validator.call_args.args, ('8.8.8.8:1080',))
            self.assertEqual(validator.call_args.kwargs['proxy_url'], 'socks5h://updated-user:updated-password@8.8.8.8:1080')
        self.assertEqual([item['status'] for item in service.dispatch('list_imported_proxies')['items']], ['available', 'failed'])

    def test_active_imported_pool_cannot_be_reconfigured_or_removed(self):
        service = self.service()
        result = service.dispatch('import_proxies', {'text': json.dumps([fixture()])})
        service.dispatch('configure_proxy', {'enabled': True, 'source': 'imported'})
        service.manager.active = True
        with self.assertRaises(ServiceError):
            service.dispatch('remove_imported_proxies', {'ids': [result['items'][0]['id']]})
        with self.assertRaises(ServiceError):
            service.dispatch('configure_proxy', {'enabled': True, 'source': 'public'})
        self.assertEqual(service.dispatch('list_imported_proxies')['total'], 1)
        service.manager.active = False

    def test_update_preserves_order_and_credentials_after_restart(self):
        service = self.service()
        originals = [fixture(), fixture('1.1.1.1')]
        service.dispatch('import_proxies', {'text': json.dumps(originals)})
        updates = [{**originals[1], 'protocol': 'http', 'password': 'replacement'}, fixture('9.9.9.9')]
        result = service.dispatch('import_proxies', {'text': json.dumps(updates)})
        self.assertEqual((result['valid'], result['added'], result['updated']), (2, 1, 1))
        self.assertTrue(service.shutdown()['closed'])
        restarted = self.service()
        rows = restarted.dispatch('list_imported_proxies')['items']
        self.assertEqual([row['endpoint'] for row in rows], ['8.8.8.8:1080', '1.1.1.1:1080', '9.9.9.9:1080'])
        restarted.dispatch('configure_proxy', {'enabled': True, 'source': 'imported'})
        with patch('proxy_pool.validate_proxy', return_value={'ok': True, 'status_code': 200}) as validator:
            restarted.dispatch('refresh_proxies')
            until(lambda: restarted.dispatch('proxy_status')['checks_total'] == 3)
            validator.assert_any_call('1.1.1.1:1080', proxy_url='http://fixture-user:replacement@1.1.1.1:1080')

    def test_unchanged_and_metadata_only_imports_keep_failure_until_route_changes(self):
        service = self.service()
        original = fixture()
        service.dispatch('import_proxies', {'text': json.dumps([original])})
        service.dispatch('configure_proxy', {'enabled': False, 'source': 'imported'})
        with patch('proxy_pool.validate_proxy', return_value={'ok': False, 'status_code': 407}):
            service.dispatch('refresh_proxies')
            until(lambda: service.dispatch('proxy_status')['checks_total'] == 1)
        same = service.dispatch('import_proxies', {'text': json.dumps([original])})
        self.assertEqual((same['updated'], same['duplicates']), (0, 1))
        metadata = {**original, 'city': '新城市'}
        changed = service.dispatch('import_proxies', {'text': json.dumps([metadata])})
        self.assertEqual(changed['updated'], 1)
        self.assertEqual(service.dispatch('list_imported_proxies')['items'][0]['status'], 'failed')
        protocol = {**metadata, 'protocol': 'http'}
        service.dispatch('import_proxies', {'text': json.dumps([protocol])})
        self.assertEqual(service.dispatch('list_imported_proxies')['items'][0]['status'], 'unchecked')

    def test_failed_persistence_keeps_old_credentials_and_failure(self):
        service = self.service()
        original = fixture()
        service.dispatch('import_proxies', {'text': json.dumps([original])})
        service.dispatch('configure_proxy', {'enabled': True, 'source': 'imported'})
        with patch('proxy_pool.validate_proxy', return_value={'ok': False, 'status_code': 407}):
            service.dispatch('refresh_proxies')
            until(lambda: service.dispatch('proxy_status')['checks_total'] == 1)
        with patch('backend_service.save_config', side_effect=ConfigError('synthetic storage failure')):
            with self.assertRaises(ConfigError):
                service.dispatch('import_proxies', {'text': json.dumps([{**original, 'password': 'replacement'}])})
        self.assertEqual(service.dispatch('list_imported_proxies')['items'][0]['status'], 'failed')
        unchanged = service.dispatch('preview_proxies', {'text': json.dumps([original])})
        self.assertEqual((unchanged['valid'], unchanged['duplicates']), (0, 1))

    def test_imported_credential_update_preserves_public_failure_for_same_endpoint(self):
        with patch('proxy_pool.fetch_candidates', return_value=['8.8.8.8:1080']), patch('proxy_pool.validate_proxy', return_value={'ok': False, 'status_code': 407}):
            service = self.service()
            service.dispatch('refresh_proxies')
            until(lambda: service.dispatch('proxy_status')['checks_total'] == 1)
        original = fixture()
        service.dispatch('import_proxies', {'text': json.dumps([original])})
        service.dispatch('configure_proxy', {'enabled': True, 'source': 'imported'})
        with patch('proxy_pool.validate_proxy', return_value={'ok': False, 'status_code': 407}):
            service.dispatch('refresh_proxies')
            until(lambda: service.dispatch('proxy_status')['checks_total'] == 1)
        service.dispatch('import_proxies', {'text': json.dumps([{**original, 'password': 'replacement'}])})
        self.assertEqual(service.dispatch('list_imported_proxies')['items'][0]['status'], 'unchecked')
        service.dispatch('configure_proxy', {'enabled': True, 'source': 'public'})
        self.assertEqual(service.dispatch('proxy_status')['failed'], 1)

    def test_old_inflight_check_cannot_restore_failure_after_credential_update(self):
        for enabled in (False, True):
            with self.subTest(enabled=enabled):
                directory = self.directory / str(enabled)
                service = BackendService(directory, manager_factory=FakeManager)
                self.addCleanup(service.shutdown)
                original = fixture()
                service.dispatch('import_proxies', {'text': json.dumps([original])})
                service.dispatch('configure_proxy', {'enabled': enabled, 'source': 'imported'})
                started, release = threading.Event(), threading.Event()
                def delayed_check(endpoint, proxy_url=None):
                    started.set()
                    release.wait(3)
                    return {'ok': False, 'status_code': 407}
                with patch('proxy_pool.validate_proxy', side_effect=delayed_check):
                    try:
                        service.dispatch('refresh_proxies')
                        self.assertTrue(started.wait(2))
                        old_pool = service.manager.pool if enabled else service._prepared_proxy_pool
                        service.dispatch('import_proxies', {'text': json.dumps([{**original, 'password': 'replacement'}])})
                        with patch('proxy_pool.validate_proxy', return_value={'ok': True, 'status_code': 200}) as validator:
                            service.dispatch('refresh_proxies')
                            until(lambda: service.dispatch('proxy_status')['checks_total'] == 1)
                            self.assertEqual(validator.call_args.kwargs['proxy_url'], 'socks5h://fixture-user:replacement@8.8.8.8:1080')
                    finally:
                        release.set()
                    until(lambda: old_pool.snapshot()['checks_total'] == 1)
                self.assertEqual(service.dispatch('proxy_status')['failed'], 0)
                self.assertEqual(service.dispatch('list_imported_proxies')['items'][0]['status'], 'available')


if __name__ == '__main__':
    unittest.main()
