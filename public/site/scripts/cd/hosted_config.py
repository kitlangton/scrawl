"""Render deployment-only configuration; no provisioning, migrations, or secrets."""
import json
import os
from pathlib import Path
import re


def worker_name(env):
    name = env.get('HOSTED_WORKER_NAME', '')
    if re.fullmatch('[a-z0-9][a-z0-9-]{1,61}[a-z0-9]', name) is None:
        raise ValueError('Missing or invalid HOSTED_WORKER_NAME')
    return name


def config(env):
    def value(name, pattern):
        result = env.get(name, '')
        if re.fullmatch(pattern, result) is None:
            raise ValueError(f'Missing or invalid {name}')
        return result

    account = value('CLOUDFLARE_ACCOUNT_ID', '[0-9a-f]{32}')
    database = value('HOSTED_D1_ID', '[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}')
    if database == '00000000-0000-0000-0000-000000000000':
        raise ValueError('Placeholder D1 ID')
    bucket = value('HOSTED_R2_BUCKET', '[a-z0-9][a-z0-9-]{1,61}[a-z0-9]')
    database_name = value('HOSTED_D1_NAME', '[a-z0-9][a-z0-9-]{1,61}[a-z0-9]')
    analytics_enabled = env.get('POSTHOG_ENABLED', 'true')
    if analytics_enabled not in ('true', 'false'):
        raise ValueError('Invalid POSTHOG_ENABLED')
    sharing_enabled = env.get('SHARING_ENABLED', 'true')
    if sharing_enabled not in ('true', 'false'):
        raise ValueError('Invalid SHARING_ENABLED')
    return {
        'name': worker_name(env), 'account_id': account,
        'main': 'dist/worker.js', 'compatibility_date': '2026-02-05',
        'compatibility_flags': ['nodejs_compat'],
        'workers_dev': False, 'preview_urls': False,
        'routes': [{'pattern': 'hosted.tryapparent.com/' + path, 'zone_name': 'tryapparent.com'}
                   for path in ('auth/*', 'v1/*', 's/*', 'share-view.js', 'share-view.css',
                                'share-wordmark.svg', 'share-font.ttf')],
        # Keep existing provider-managed plain variables and all secret bindings.
        'keep_vars': True,
        'vars': {'API_ORIGIN': 'https://hosted.tryapparent.com',
                 'SITE_ORIGIN': 'https://app.tryapparent.com',
                 'LEGACY_SITE_ORIGIN': 'https://hosted.tryapparent.com',
                 'SHARING_ENABLED': sharing_enabled,
                 'POSTHOG_ENABLED': analytics_enabled,
                 'POSTHOG_HOST': 'https://eu.i.posthog.com',
                 'POSTHOG_ENVIRONMENT': 'production'},
        'd1_databases': [{'binding': 'DB', 'database_name': database_name,
                          'database_id': database, 'migrations_dir': 'migrations'}],
        'r2_buckets': [{'binding': 'BUCKET', 'bucket_name': bucket}],
        'triggers': {'crons': ['*/15 * * * *']}, 'observability': {'enabled': False},
    }


if __name__ == '__main__':
    # Kept inside connect/service/ so Wrangler resolves entrypoint/migrations correctly.
    Path('connect/service/wrangler.cd.json').write_text(json.dumps(config(os.environ), indent=2) + '\n')
