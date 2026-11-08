"""Render deployment-only configuration; no provisioning, migrations, or secrets."""
import json
import os
from pathlib import Path
import re


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
    client = value('HOSTED_GOOGLE_CLIENT_ID', '[A-Za-z0-9_-]+\\.apps\\.googleusercontent\\.com')
    return {
        'name': 'apparent-hosted', 'account_id': account,
        'main': 'dist/worker.js', 'compatibility_date': '2026-02-05',
        'workers_dev': False, 'preview_urls': False,
        'routes': [{'pattern': 'hosted.tryapparent.com', 'custom_domain': True}],
        'vars': {'API_ORIGIN': 'https://hosted.tryapparent.com',
                 'SITE_ORIGIN': 'https://app.tryapparent.com',
                 'LEGACY_SITE_ORIGIN': 'https://hosted.tryapparent.com',
                 'GOOGLE_CLIENT_ID': client},
        'd1_databases': [{'binding': 'DB', 'database_name': 'apparent-hosted',
                          'database_id': database, 'migrations_dir': 'migrations'}],
        'r2_buckets': [{'binding': 'BUCKET', 'bucket_name': bucket}],
        'triggers': {'crons': ['*/15 * * * *']}, 'observability': {'enabled': False},
    }


if __name__ == '__main__':
    # Kept inside connect/service/ so Wrangler resolves entrypoint/migrations correctly.
    Path('connect/service/wrangler.cd.json').write_text(json.dumps(config(os.environ), indent=2) + '\n')
