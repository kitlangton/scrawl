"""Offline deployment configuration and Cloudflare readback contracts."""
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch


def load(name):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + '.py'))
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


configuration = load('hosted_config')
worker = load('verify_worker')


class HostedDeliveryTests(unittest.TestCase):
    def setUp(self):
        self.env = {'CLOUDFLARE_ACCOUNT_ID': 'a' * 32,
                    'HOSTED_D1_ID': '11111111-1111-1111-1111-111111111111',
                    'HOSTED_R2_BUCKET': 'synthetic-private-bucket',
                    'HOSTED_GOOGLE_CLIENT_ID': 'synthetic.apps.googleusercontent.com'}

    def test_missing_invalid_or_placeholder_inputs_fail_closed(self):
        for key in self.env:
            with self.subTest(key=key), self.assertRaises(ValueError):
                configuration.config({**self.env, key: ''})
        for value in ['00000000-0000-0000-0000-000000000000', 'bad\"\nconfig']:
            with self.assertRaises(ValueError):
                configuration.config({**self.env, 'HOSTED_D1_ID': value})

    def test_production_topology_has_no_secret_or_paid_override(self):
        result = configuration.config(self.env)
        self.assertEqual(result['vars']['API_ORIGIN'], 'https://hosted.tryapparent.com')
        self.assertEqual(result['vars']['SITE_ORIGIN'], 'https://app.tryapparent.com')
        self.assertEqual(result['routes'], [{'pattern': 'hosted.tryapparent.com', 'custom_domain': True}])
        self.assertFalse(result['workers_dev'])
        self.assertFalse(result['observability']['enabled'])
        self.assertNotIn('limits', result)
        self.assertNotIn('SECRET', json.dumps(result))
        self.assertEqual(result['d1_databases'][0]['database_id'], self.env['HOSTED_D1_ID'])

    def test_readback_rejects_stale_or_partial_deployment_before_public_request(self):
        deployment = {'deployments': [{'id': 'deployment', 'created_on': '2026-01-01',
                                      'versions': [{'version_id': 'version', 'percentage': 100}]}]}
        with patch.object(worker, 'api', side_effect=[deployment, {'annotations': {'workers/tag': 'old'}}]), patch.object(worker, 'urlopen') as http:
            with self.assertRaisesRegex(ValueError, 'not the checked commit'):
                worker.verify('a' * 40)
            http.assert_not_called()
        deployment['deployments'][0]['versions'][0]['percentage'] = 50
        with patch.object(worker, 'api', return_value=deployment), self.assertRaisesRegex(ValueError, 'fully deployed'):
            worker.verify('a' * 40)

    def test_matching_version_requires_anonymous_credentialed_cors(self):
        from io import BytesIO
        sha = 'a' * 40
        deployment = {'deployments': [{'id': 'deployment', 'created_on': '2026-01-01',
                                      'versions': [{'version_id': 'version', 'percentage': 100}]}]}
        for origin, authenticated, succeeds in [('https://app.tryapparent.com', False, True),
                                                ('https://evil.example', False, False),
                                                ('https://app.tryapparent.com', True, False)]:
            class Response(BytesIO):
                headers = {'Access-Control-Allow-Origin': origin, 'Access-Control-Allow-Credentials': 'true'}
            response = Response(json.dumps({'authenticated': authenticated}).encode())
            with patch.object(worker, 'api', side_effect=[deployment, {'id': 'version', 'annotations': {'workers/tag': sha}}]), patch.object(worker, 'urlopen', return_value=response):
                if succeeds:
                    worker.verify(sha)
                else:
                    with self.assertRaises(ValueError):
                        worker.verify(sha)


if __name__ == '__main__':
    unittest.main()
