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
                    'HOSTED_WORKER_NAME': 'synthetic-existing-worker',
                    'HOSTED_D1_NAME': 'synthetic-existing-database'}
        environment = patch.dict(worker.os.environ, self.env)
        environment.start()
        self.addCleanup(environment.stop)

    def test_readback_uses_selected_existing_worker_and_descriptive_client(self):
        from io import BytesIO
        response = BytesIO(b'{"success":true,"result":{}}')
        with patch.dict(worker.os.environ, {'CLOUDFLARE_ACCOUNT_ID': 'a' * 32,
                                           'CLOUDFLARE_API_TOKEN': 'synthetic-test-only',
                                           'HOSTED_WORKER_NAME': 'synthetic-existing-worker'}), patch.object(worker, 'urlopen', return_value=response) as http:
            worker.api('deployments')
        request = http.call_args.args[0]
        self.assertIn('/workers/scripts/synthetic-existing-worker/deployments', request.full_url)
        self.assertEqual(request.get_header('User-agent'), 'apparent-connect-cd/1')

    def test_analytics_defaults_on_without_exposing_secrets_and_preserves_explicit_stop(self):
        rendered = configuration.config(self.env)
        self.assertEqual(rendered['vars']['POSTHOG_ENABLED'], 'true')
        self.assertEqual(rendered['vars']['POSTHOG_HOST'], 'https://eu.i.posthog.com')
        self.assertEqual(rendered['vars']['POSTHOG_ENVIRONMENT'], 'production')
        self.assertNotIn('POSTHOG_PROJECT_TOKEN', json.dumps(rendered))
        self.assertTrue(rendered['keep_vars'])
        self.assertEqual(configuration.config({**self.env, 'POSTHOG_ENABLED': 'false'})['vars']['POSTHOG_ENABLED'], 'false')
        with self.assertRaises(ValueError):
            configuration.config({**self.env, 'POSTHOG_ENABLED': 'invalid'})

    def test_existing_service_need_not_already_have_new_analytics_variables(self):
        settings = self.existing_settings()
        settings['bindings'] = [b for b in settings['bindings'] if not b['name'].startswith('POSTHOG_')]
        with patch.object(worker, 'api', side_effect=[settings, {'schedules': [{'cron': '*/15 * * * *'}]}]), patch.object(worker, 'provider_api', side_effect=self.route_readback()):
            worker.preflight()

    def existing_settings(self):
        config = configuration.config(self.env)
        return {'bindings': [
            {'name': 'DB', 'type': 'd1', 'id': self.env['HOSTED_D1_ID']},
            {'name': 'BUCKET', 'type': 'r2_bucket', 'bucket_name': self.env['HOSTED_R2_BUCKET']},
            *[{'name': key, 'type': 'plain_text', 'text': value} for key, value in config['vars'].items()],
            *[{'name': name, 'type': 'secret_text'} for name in ('GOOGLE_CLIENT_ID', 'GOOGLE_CLIENT_SECRET', 'BETTER_AUTH_SECRET', 'TOKEN_ENCRYPTION_KEY')],
        ]}

    def test_preflight_accepts_exact_existing_bindings_and_preserves_other_settings(self):
        settings = self.existing_settings()
        settings['bindings'].extend([{'name': 'OTHER_WRITER', 'type': 'plain_text', 'text': 'preserve'},
                                     {'name': 'OTHER_SECRET', 'type': 'secret_text'}])
        with patch.object(worker, 'api', side_effect=[settings, {'schedules': [{'cron': '*/15 * * * *'}]}]) as api, patch.object(worker, 'provider_api', side_effect=self.route_readback()):
            worker.preflight()
        self.assertEqual([c.args[0] for c in api.call_args_list], ['settings', 'schedules'])

    def route_readback(self):
        return [[{'id': 'b' * 32, 'name': 'tryapparent.com', 'account': {'id': self.env['CLOUDFLARE_ACCOUNT_ID']}}],
                [{'pattern': r['pattern'], 'script': self.env['HOSTED_WORKER_NAME']}
                 for r in configuration.config(self.env)['routes']]]

    def test_incomplete_settings_fail_before_publication_ordering(self):
        for settings in (None, {}, {'bindings': None}, {'bindings': {}}, {'bindings': [None]},
                         {'bindings': [{'name': 'DB'}]}):
            with self.subTest(settings=settings), patch.object(worker, 'api', return_value=settings), patch.object(worker, 'publication_needed') as order:
                with self.assertRaises(ValueError):
                    worker.can_publish('a' * 40)
                order.assert_not_called()

    def test_route_and_cron_drift_fail_before_publication_ordering(self):
        import copy
        for schedules in (None, {}, {'schedules': None}, {'schedules': [None]},
                          {'schedules': [{'cron': '* * * * *'}]}):
            with patch.object(worker, 'api', side_effect=[self.existing_settings(), schedules]), patch.object(worker, 'publication_needed') as order:
                with self.assertRaises(ValueError):
                    worker.can_publish('a' * 40)
                order.assert_not_called()
        cases = [[None], [[], []], [self.route_readback()[0], None],
                 [self.route_readback()[0], []], [self.route_readback()[0], [None]]]
        drift = copy.deepcopy(self.route_readback())
        drift[1][0]['script'] = 'another-writer'
        cases.append(drift)
        extra = copy.deepcopy(self.route_readback())
        extra[1].append({'pattern': 'hosted.tryapparent.com/s/*', 'script': self.env['HOSTED_WORKER_NAME']})
        cases.append(extra)
        for routes in cases:
            with patch.object(worker, 'api', side_effect=[self.existing_settings(), {'schedules': [{'cron': '*/15 * * * *'}]}]), patch.object(worker, 'provider_api', side_effect=routes), patch.object(worker, 'publication_needed') as order:
                with self.assertRaises(ValueError):
                    worker.can_publish('a' * 40)
                order.assert_not_called()

    def test_missing_service_or_drift_stops_before_publication_ordering(self):
        from urllib.error import HTTPError
        import copy
        changes = [('DB', 'id', 'different'), ('BUCKET', 'bucket_name', 'other'),
                   ('SITE_ORIGIN', 'text', 'https://foreign.example'),
                   ('GOOGLE_CLIENT_SECRET', 'type', 'plain_text'), ('GOOGLE_CLIENT_ID', 'type', 'service')]
        cases = []
        for name, field, value in changes:
            settings = copy.deepcopy(self.existing_settings())
            next(b for b in settings['bindings'] if b['name'] == name)[field] = value
            cases.append(settings)
        unknown = self.existing_settings()
        unknown['bindings'].append({'name': 'OTHER_WRITER_KV', 'type': 'kv_namespace', 'namespace_id': 'preserve'})
        cases.append(unknown)
        for settings in cases:
            with patch.object(worker, 'api', return_value=settings), patch.object(worker, 'publication_needed') as order:
                with self.assertRaises(ValueError):
                    worker.can_publish('a' * 40)
                order.assert_not_called()
        with patch.object(worker, 'api', side_effect=HTTPError('synthetic', 404, 'missing', {}, None)), patch.object(worker, 'publication_needed') as order:
            with self.assertRaises(HTTPError):
                worker.can_publish('a' * 40)
            order.assert_not_called()

    def test_selected_name_is_validated_before_any_provider_request(self):
        for key in ('HOSTED_WORKER_NAME', 'CLOUDFLARE_ACCOUNT_ID'):
            for value in ('', '../other', 'name/path', 'bad name'):
                with patch.dict(worker.os.environ, {key: value}), patch.object(worker, 'urlopen') as http:
                    with self.assertRaises(ValueError):
                        worker.api('settings')
                    http.assert_not_called()

    def test_workflow_keeps_activation_gate_and_orders_existing_service_preflight(self):
        text = (Path(__file__).resolve().parents[4] / '.github/workflows/web-delivery.yml').read_text()
        hosted = text.split('  hosted:\n', 1)[1]
        self.assertIn("vars.HOSTED_CD_ENABLED == 'true'", hosted)
        self.assertIn('HOSTED_WORKER_NAME: ${{ vars.HOSTED_WORKER_NAME }}', hosted)
        self.assertIn('HOSTED_D1_NAME: ${{ vars.HOSTED_D1_NAME }}', hosted)
        self.assertNotIn('HOSTED_GOOGLE_CLIENT_ID:', hosted)
        self.assertLess(hosted.index('verify_worker.py --publication-needed'), hosted.index('wrangler deploy'))
        self.assertIn("if: steps.order.outputs.publish == 'true'", hosted)

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
        self.assertEqual(result['name'], self.env['HOSTED_WORKER_NAME'])
        self.assertEqual(result['routes'], [{'pattern': 'hosted.tryapparent.com/' + path + '/*', 'zone_name': 'tryapparent.com'} for path in ('auth', 'v1')])
        self.assertTrue(result['keep_vars'])
        self.assertNotIn('GOOGLE_CLIENT_ID', result['vars'])
        self.assertEqual(result['d1_databases'][0]['database_name'], self.env['HOSTED_D1_NAME'])
        self.assertEqual(result['compatibility_flags'], ['nodejs_compat'])
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

    def test_publication_uses_current_worker_source_identity(self):
        deployment = {'deployments': [{'id': 'deployment', 'created_on': '2026-11-03',
                                      'versions': [{'version_id': 'version', 'percentage': 100}]}]}
        for allowed in (True, False):
            with patch.object(worker, 'preflight') as preflight, patch.object(worker, 'api', side_effect=[deployment, {'annotations': {'workers/tag': 'a' * 40}}]), patch.dict(worker.os.environ, {'GITHUB_REPOSITORY': 'example/project'}), patch.object(worker, 'publication_needed', return_value=allowed) as order:
                self.assertEqual(worker.can_publish('b' * 40), allowed)
                order.assert_called_once_with('example/project', 'b' * 40, 'a' * 40)
                preflight.assert_called_once_with()

    def test_legacy_publication_tag_still_requires_explicit_baseline(self):
        deployment = {'deployments': [{'id': 'deployment', 'created_on': '2026-11-03',
                                      'versions': [{'version_id': 'version', 'percentage': 100}]}]}
        with patch.object(worker, 'preflight'), patch.object(worker, 'api', side_effect=[deployment, {'annotations': {'workers/tag': 'legacy-human-label'}}]), patch.dict(worker.os.environ, {'GITHUB_REPOSITORY': 'example/project'}):
            with self.assertRaisesRegex(ValueError, 'Unknown publication revision'):
                worker.can_publish('a' * 40)

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
            with patch.object(worker, 'api', side_effect=[deployment, {'id': 'version', 'annotations': {'workers/tag': sha}}]), patch.object(worker, 'urlopen', return_value=response) as http:
                if succeeds:
                    worker.verify(sha)
                    self.assertEqual(http.call_args.args[0].get_header('User-agent'), 'apparent-connect-cd/1')
                else:
                    with self.assertRaises(ValueError):
                        worker.verify(sha)


if __name__ == '__main__':
    unittest.main()
