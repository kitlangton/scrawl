"""Read back the deployed Worker version and anonymous cookie/CORS boundary."""
import json
import os
import re
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[4] / "enablement"))
from release_gate import publication_needed
from hosted_config import config, worker_name
from urllib.request import Request, urlopen


def provider_api(path):
    request = Request('https://api.cloudflare.com/client/v4/' + path,
                      headers={'Authorization': 'Bearer ' + os.environ['CLOUDFLARE_API_TOKEN'],
                               'User-Agent': 'apparent-connect-cd/1'})
    with urlopen(request, timeout=30) as response:
        result = json.load(response)
    if not isinstance(result, dict) or result.get('success') is not True or 'result' not in result:
        raise ValueError('Cloudflare readback failed')
    return result['result']


def account_id():
    account = os.environ.get('CLOUDFLARE_ACCOUNT_ID', '')
    if re.fullmatch('[0-9a-f]{32}', account) is None:
        raise ValueError('Missing or invalid CLOUDFLARE_ACCOUNT_ID')
    return account


def api(path):
    return provider_api('accounts/' + account_id() + '/workers/scripts/' +
                        worker_name(os.environ) + '/' + path)


def deployed_revision():
    deployments = api('deployments')['deployments']
    latest = max(deployments, key=lambda d: d['created_on'])
    versions = latest['versions']
    if len(versions) != 1 or versions[0]['percentage'] != 100:
        raise ValueError('Expected a single fully deployed version')
    version = api('versions/' + versions[0]['version_id'])
    return latest, version


def verify(sha):
    latest, version = deployed_revision()
    if version.get('annotations', {}).get('workers/tag') != sha:
        raise ValueError('Deployed Worker is not the checked commit')
    request = Request('https://hosted.tryapparent.com/v1/session',
                      headers={'Origin': 'https://app.tryapparent.com',
                               'User-Agent': 'apparent-connect-cd/1'})
    with urlopen(request, timeout=30) as response:
        session = json.load(response)
        if session.get('authenticated') is not False:
            raise ValueError('Anonymous session contract failed')
        if response.headers.get('Access-Control-Allow-Origin') != 'https://app.tryapparent.com':
            raise ValueError('Canonical CORS origin not applied')
        if response.headers.get('Access-Control-Allow-Credentials') != 'true':
            raise ValueError('Credentialed CORS not applied')
    print(json.dumps({'sha': sha, 'deployment': latest['id'], 'version': version['id']}))


def preflight():
    # Existing-service admission only. Never create a replacement or silently
    # rebind a live service to another database/bucket or discard a writer's binding.
    desired = config(os.environ)
    settings = api('settings')
    if not isinstance(settings, dict) or not isinstance(settings.get('bindings'), list):
        raise ValueError('Incomplete existing Worker settings')
    bindings = settings['bindings']
    if any(not isinstance(b, dict) or not isinstance(b.get('name'), str) or
           not isinstance(b.get('type'), str) for b in bindings):
        raise ValueError('Malformed existing Worker bindings')
    if len({b['name'] for b in bindings}) != len(bindings):
        raise ValueError('Duplicate existing Worker binding')
    by_name = {binding['name']: binding for binding in bindings}
    expected = [('DB', 'd1', 'id', desired['d1_databases'][0]['database_id']),
                ('BUCKET', 'r2_bucket', 'bucket_name', desired['r2_buckets'][0]['bucket_name'])]
    for name, kind, field, value in expected:
        binding = by_name.get(name, {})
        if binding.get('type') != kind or binding.get(field) != value:
            raise ValueError('Existing ' + name + ' binding differs from approved input')
    # Existing identity/origins must match before deployment. New analytics
    # defaults are deployment inputs, not prerequisites on the old version.
    for name in ('API_ORIGIN', 'SITE_ORIGIN', 'LEGACY_SITE_ORIGIN'):
        value = desired['vars'][name]
        binding = by_name.get(name, {})
        if binding.get('type') != 'plain_text' or binding.get('text') != value:
            raise ValueError('Existing origin binding differs from approved input')
    for name in ('GOOGLE_CLIENT_SECRET', 'BETTER_AUTH_SECRET', 'TOKEN_ENCRYPTION_KEY'):
        if by_name.get(name, {}).get('type') != 'secret_text':
            raise ValueError('Required existing secret binding is absent')
    if by_name.get('GOOGLE_CLIENT_ID', {}).get('type') not in ('plain_text', 'secret_text'):
        raise ValueError('Required existing Google client binding is absent')
    if any(b.get('type') not in ('plain_text', 'secret_text') and b.get('name') not in ('DB', 'BUCKET') for b in bindings):
        raise ValueError('Unreviewed existing binding would be removed')

    schedules = api('schedules')
    if (not isinstance(schedules, dict) or not isinstance(schedules.get('schedules'), list) or
            any(not isinstance(s, dict) or not isinstance(s.get('cron'), str)
                for s in schedules['schedules']) or
            sorted(s['cron'] for s in schedules['schedules']) != sorted(desired['triggers']['crons'])):
        raise ValueError('Existing cron schedule differs from approved input')
    zones = provider_api('zones?name=tryapparent.com')
    if (not isinstance(zones, list) or len(zones) != 1 or not isinstance(zones[0], dict) or
            zones[0].get('name') != 'tryapparent.com' or
            zones[0].get('account', {}).get('id') != account_id() or
            re.fullmatch('[0-9a-f]{32}', zones[0].get('id', '')) is None):
        raise ValueError('Existing route zone is not established')
    routes = provider_api('zones/' + zones[0]['id'] + '/workers/routes')
    if (not isinstance(routes, list) or
            any(not isinstance(r, dict) or not isinstance(r.get('pattern'), str) for r in routes)):
        raise ValueError('Incomplete existing route readback')
    selected = sorted(r['pattern'] for r in routes if r.get('script') == desired['name'])
    if selected != sorted(r['pattern'] for r in desired['routes']):
        raise ValueError('Existing Worker routes differ from approved input')


def can_publish(sha):
    preflight()
    _, version = deployed_revision()
    # Unknown/mixed deployment identities fail closed, never silently replace.
    published = version.get('annotations', {}).get('workers/tag', '')
    return publication_needed(os.environ['GITHUB_REPOSITORY'], sha, published)


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == '--publication-needed':
        needed = can_publish(sys.argv[2])
        with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
            output.write(f'publish={str(needed).lower()}\n')
    else:
        verify(sys.argv[1])
