"""Subscription fixtures and mocked transports; no user data or live network."""
import json
from pathlib import Path
import socket
import sys
import threading
import time
import unittest
from unittest.mock import MagicMock, patch

import requests

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'python' / 'collector'))
from proxy_subscription import (MAX_RESPONSE_BYTES, SubscriptionCancelled, SubscriptionError,
                                fetch_subscription, subscription_summary, validate_subscription_url)


class SubscriptionTests(unittest.TestCase):
    def setUp(self):
        self.session = MagicMock()
        self.session.__enter__.return_value = self.session
        self.response = self.session.get.return_value.__enter__.return_value
        self.response.status_code = 200
        self.response.headers = {}
        self.response.iter_content.return_value = [b'8.8.8.8:80']
        sessions = patch('proxy_subscription.requests.Session', return_value=self.session)
        self.sessions = sessions.start()
        self.addCleanup(sessions.stop)
        resolver = patch('proxy_subscription.socket.getaddrinfo', return_value=[
            (socket.AF_INET, socket.SOCK_STREAM, 6, '', ('8.8.4.4', 443))])
        self.resolver = resolver.start()
        self.addCleanup(resolver.stop)

    def test_geonode_candidates_are_normalized_without_trusting_supplier_health(self):
        payload = {'data': [
            {'ip': '8.8.8.8', 'port': '1080', 'protocols': ['http', 'socks5'], 'status': 'available'},
            {'ip': '1.1.1.1', 'port': '8080', 'protocols': ['https']},
            {'ip': '9.9.9.9', 'port': '1080', 'protocols': ['socks4']},
        ]}
        session = MagicMock()
        session.__enter__.return_value = session
        response = session.get.return_value.__enter__.return_value
        response.status_code = 200
        response.headers = {}
        response.iter_content.return_value = [json.dumps(payload).encode()]
        addresses = [(socket.AF_INET, socket.SOCK_STREAM, 6, '', ('8.8.4.4', 443))]
        with patch('proxy_subscription.requests.Session', return_value=session), patch('proxy_subscription.socket.getaddrinfo', return_value=addresses):
            result = fetch_subscription('https://proxylist.geonode.com/api/proxy-list?token=synthetic-secret')
        self.assertEqual((result['count'], result['rejected'], result['duplicates']), (2, 1, 0))
        self.assertEqual([item['protocol'] for item in result['records']], ['socks5h', 'http'])
        self.assertTrue(all(item['username'] == item['password'] == '' for item in result['records']))
        self.assertFalse(session.trust_env)
        self.assertTrue(session.get.call_args.kwargs['verify'])
        self.assertFalse(session.get.call_args.kwargs['allow_redirects'])
        self.assertNotIn('synthetic-secret', json.dumps(result['source']))
        self.assertEqual(subscription_summary('https://example.com/private/token?key=secret'),
                         {'host': 'example.com', 'port': 443, 'label': 'example.com', 'has_query': True})

    def test_json_array_ipv6_endpoint_preserves_explicit_authentication(self):
        self.response.iter_content.return_value = [json.dumps([{
            'endpoint': '[2606:4700:4700::1111]:1080', 'protocol': 'socks5',
            'username': 'synthetic-user', 'password': 'synthetic-pass'}, {'endpoint': '8.8.8.8:80'}]).encode()]
        result = fetch_subscription('https://example.com/list')
        self.assertEqual(result['count'], 2)
        self.assertEqual(result['records'][0]['password'], 'synthetic-pass')

    def test_cancel_requested_while_receiving_does_not_return_success(self):
        stop = threading.Event()
        def chunks():
            time.sleep(0.02)
            stop.set()
            yield b'8.8.8.8:80'
        self.response.iter_content.side_effect = lambda size: chunks()
        with self.assertRaises(SubscriptionCancelled):
            fetch_subscription('https://example.com/list', stop)

    def test_summary_is_local_and_redacts_path_query_and_fragment_secrets(self):
        self.assertEqual(subscription_summary('https://EXAMPLE.com:8443/synthetic-token/list?key=secret'),
                         {'host': 'example.com', 'port': 8443, 'label': 'example.com:8443', 'has_query': True})
        self.resolver.assert_not_called()
        self.sessions.assert_not_called()

    def test_url_rejects_unsafe_schemes_credentials_local_hosts_and_ambiguous_authorities(self):
        urls = ['http://example.com/list', 'file:///list', 'https://user:secret@example.com/list',
                'https://localhost/', 'https://a.localhost/', 'https://router.local/',
                'https://127.0.0.1/', 'https://192.168.1.1/', 'https://169.254.169.254/',
                'https://100.64.0.1/', 'https://[::1]/', 'https://[::ffff:127.0.0.1]/',
                'https://224.0.0.1/', 'https://0.0.0.0/', 'https://2130706433/',
                'https://example.com:0/', 'https://example.com:65536/',
                'https://example.com/#secret', 'https://example.com\\@127.0.0.1/',
                'https://example.com/%0a\nsecret', 'https://example.com/%2f#secret']
        for url in urls:
            with self.subTest(case=urls.index(url)), self.assertRaises(SubscriptionError) as caught:
                subscription_summary(url)
            self.assertNotIn('secret', str(caught.exception))
        self.resolver.assert_not_called()
        self.sessions.assert_not_called()

    def test_idna_and_public_ipv6_urls_normalize_without_network_for_summary(self):
        summary = subscription_summary('https://例子.com/list')
        self.assertEqual(summary['host'], 'xn--fsqu00a.com')
        self.assertEqual(validate_subscription_url('HTTPS://EXAMPLE.COM:443/list?token=original'),
                         'https://example.com/list?token=original')
        self.assertEqual(subscription_summary('https://[2606:4700:4700::1111]:8443/list')['label'],
                         '[2606:4700:4700::1111]:8443')

    def test_dns_rejects_mixed_public_and_private_answers_without_http(self):
        self.resolver.return_value.append((socket.AF_INET, socket.SOCK_STREAM, 6, '', ('127.0.0.1', 443)))
        with self.assertRaises(SubscriptionError):
            fetch_subscription('https://example.com/list?token=secret')
        self.sessions.assert_not_called()

    def test_dns_failure_does_not_echo_query_or_exception(self):
        self.resolver.side_effect = socket.gaierror('synthetic-secret failed')
        with self.assertRaises(SubscriptionError) as caught:
            fetch_subscription('https://example.com/list?token=synthetic-secret')
        self.assertNotIn('synthetic-secret', str(caught.exception))
        self.sessions.assert_not_called()

    def test_connection_is_pinned_to_public_ip_with_original_host_and_tls_name(self):
        fetch_subscription('https://example.com:8443/private?token=secret')
        adapter = self.session.mount.call_args.args[1]
        prepared = requests.Request('GET', 'https://example.com:8443/private?token=secret').prepare()
        with patch.object(adapter.poolmanager, 'connection_from_host') as connect:
            adapter.get_connection_with_tls_context(prepared, verify=True, proxies={})
        kwargs = connect.call_args.kwargs
        self.assertEqual((kwargs['host'], kwargs['port'], kwargs['scheme']), ('8.8.4.4', 8443, 'https'))
        self.assertEqual(kwargs['pool_kwargs']['assert_hostname'], 'example.com')
        self.assertEqual(kwargs['pool_kwargs']['server_hostname'], 'example.com')
        self.assertEqual(self.session.get.call_args.kwargs['headers']['Host'], 'example.com:8443')
        self.assertEqual(self.session.get.call_args.kwargs['proxies'], {})
        self.assertEqual(self.session.get.call_args.kwargs['timeout'], (5, 5))
        self.assertEqual(self.resolver.call_count, 1)
        with self.assertRaises(SubscriptionError):
            adapter.get_connection_with_tls_context(prepared, verify=False)
        with self.assertRaises(SubscriptionError):
            adapter.get_connection_with_tls_context(prepared, verify=True, proxies={'https': 'http://127.0.0.1:80'})
        redirected = requests.Request('GET', 'https://other.example.com/').prepare()
        with self.assertRaises(SubscriptionError):
            adapter.get_connection_with_tls_context(redirected, verify=True)
        adapter.close()

    def test_redirect_is_rejected_without_following_location(self):
        self.response.status_code = 302
        self.response.headers = {'Location': 'http://127.0.0.1/secret'}
        with self.assertRaisesRegex(SubscriptionError, 'HTTP 302'):
            fetch_subscription('https://example.com/list')
        self.assertEqual(self.session.get.call_count, 1)
        self.response.iter_content.assert_not_called()

    def test_response_length_and_streamed_size_are_bounded(self):
        self.response.headers = {'Content-Length': str(MAX_RESPONSE_BYTES + 1)}
        with self.assertRaises(SubscriptionError):
            fetch_subscription('https://example.com/list')
        self.response.iter_content.assert_not_called()
        self.response.headers = {}
        self.response.iter_content.return_value = [b'x' * MAX_RESPONSE_BYTES, b'x']
        with self.assertRaisesRegex(SubscriptionError, '2 MiB'):
            fetch_subscription('https://example.com/list')

    def test_compression_is_not_decoded_beyond_response_limit(self):
        self.response.headers = {'Content-Encoding': 'gzip'}
        with self.assertRaises(SubscriptionError):
            fetch_subscription('https://example.com/list')
        self.assertEqual(self.session.get.call_args.kwargs['headers']['Accept-Encoding'], 'identity')
        self.response.iter_content.assert_not_called()

    def test_malformed_utf8_json_html_and_unknown_schema_do_not_echo_body(self):
        for content in (b'\xff', b'{"private-token":', b'<html>synthetic-secret</html>', b'{"proxies":[]}'):
            with self.subTest(kind=content[:1]):
                self.response.iter_content.return_value = [content]
                try:
                    result = fetch_subscription('https://example.com/list?key=synthetic-secret')
                except SubscriptionError as error:
                    self.assertNotIn('synthetic-secret', str(error))
                else:
                    self.assertEqual(result['count'], 0)
                    self.assertGreater(result['rejected'], 0)

    def test_text_and_json_duplicates_are_first_wins_and_invalid_candidates_are_counted(self):
        self.response.iter_content.return_value = [b'# ignored\n8.8.8.8:80\n8.8.8.8:80\nsocks5://user:pass@1.1.1.1:1080\n127.0.0.1:80\nsocks4://9.9.9.9:1080']
        result = fetch_subscription('https://example.com/list')
        self.assertEqual((result['count'], result['duplicates'], result['rejected']), (2, 1, 2))
        self.assertEqual(result['records'][1]['protocol'], 'socks5h')
        self.response.iter_content.return_value = [json.dumps([{'endpoint': '8.8.8.8:80'},
            {'endpoint': '8.8.8.8:80', 'username': 'later', 'password': 'ignored'}]).encode()]
        result = fetch_subscription('https://example.com/list')
        self.assertEqual((result['count'], result['duplicates']), (1, 1))
        self.assertEqual(result['records'][0]['username'], '')

    def test_geonode_does_not_infer_auth_and_skips_unknown_protocols(self):
        self.response.iter_content.return_value = [json.dumps({'data': [
            {'ip': '8.8.8.8', 'port': 80, 'protocols': ['HTTPS', 'socks4'], 'user': 'untrusted', 'password': 'ignored'},
            {'ip': '1.1.1.1', 'port': 80, 'protocols': ['ftp']},
            {'ip': '9.9.9.9', 'port': 80, 'protocols': 'http'},
            {'ip': '127.0.0.1', 'port': 80, 'protocols': ['http']},
        ]}).encode()]
        result = fetch_subscription('https://example.com/list')
        self.assertEqual((result['count'], result['rejected']), (1, 3))
        self.assertEqual(result['records'][0]['username'], '')
        self.assertEqual(result['records'][0]['password'], '')

    def test_more_than_five_thousand_candidates_fails_instead_of_truncating(self):
        for payload in ('\n'.join(['8.8.8.8:80'] * 5001), json.dumps([{'endpoint': '8.8.8.8:80'}] * 5001)):
            self.response.iter_content.return_value = [payload.encode()]
            with self.assertRaisesRegex(SubscriptionError, '5000'):
                fetch_subscription('https://example.com/list')

    def test_empty_json_list_is_valid_zero_candidate_snapshot(self):
        for content in (b'[]', b'{"data":[]}'):
            self.response.iter_content.return_value = [content]
            result = fetch_subscription('https://example.com/list')
            self.assertEqual((result['count'], result['rejected'], result['duplicates']), (0, 0, 0))

    def test_nonempty_completely_invalid_subscription_is_not_a_successful_empty_snapshot(self):
        for content in (b'<html>please login</html>', b'{"data":[{"ip":"8.8.8.8","port":80,"protocols":["socks4"]}]}'):
            self.response.iter_content.return_value = [content]
            with self.assertRaises(SubscriptionError):
                fetch_subscription('https://example.com/list')

    def test_transport_error_is_safe_and_does_not_retry(self):
        self.session.get.side_effect = requests.exceptions.SSLError('https://example.com/token=synthetic-secret')
        with self.assertRaises(SubscriptionError) as caught:
            fetch_subscription('https://example.com/token=synthetic-secret')
        self.assertNotIn('synthetic-secret', str(caught.exception))
        self.assertEqual(self.session.get.call_count, 1)

    def test_pre_cancelled_fetch_does_not_resolve_or_create_session(self):
        stop = threading.Event()
        stop.set()
        with self.assertRaises(SubscriptionCancelled):
            fetch_subscription('https://example.com/list', stop)
        self.resolver.assert_not_called()
        self.sessions.assert_not_called()

    def test_total_timeout_bounds_slow_dns_without_starting_http_after_cancel(self):
        release, ended = threading.Event(), threading.Event()
        def slow_dns(*args, **kwargs):
            try:
                release.wait(2)
                return [(socket.AF_INET, socket.SOCK_STREAM, 6, '', ('8.8.4.4', 443))]
            finally:
                ended.set()
        self.resolver.side_effect = slow_dns
        try:
            started = time.monotonic()
            with patch('proxy_subscription.FETCH_TIMEOUT_SECONDS', 0.05), self.assertRaisesRegex(SubscriptionError, '超时'):
                fetch_subscription('https://example.com/list')
            self.assertLess(time.monotonic() - started, 0.5)
        finally:
            release.set()
            self.assertTrue(ended.wait(1))
        self.sessions.assert_not_called()


if __name__ == '__main__':
    unittest.main()
