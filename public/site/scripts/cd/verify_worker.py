"""Read back the deployed Worker version and anonymous cookie/CORS boundary."""
import json
import os
import sys
from urllib.request import Request, urlopen


def api(path):
    request = Request('https://api.cloudflare.com/client/v4/accounts/' +
                      os.environ['CLOUDFLARE_ACCOUNT_ID'] + '/workers/scripts/apparent-hosted/' + path,
                      headers={'Authorization': 'Bearer ' + os.environ['CLOUDFLARE_API_TOKEN']})
    with urlopen(request, timeout=30) as response:
        result = json.load(response)
    if not result.get('success'):
        raise ValueError('Cloudflare readback failed')
    return result['result']


def verify(sha):
    deployments = api('deployments')['deployments']
    latest = max(deployments, key=lambda d: d['created_on'])
    versions = latest['versions']
    if len(versions) != 1 or versions[0]['percentage'] != 100:
        raise ValueError('Expected a single fully deployed version')
    version = api('versions/' + versions[0]['version_id'])
    if version.get('annotations', {}).get('workers/tag') != sha:
        raise ValueError('Deployed Worker is not the checked commit')
    request = Request('https://hosted.tryapparent.com/v1/session',
                      headers={'Origin': 'https://app.tryapparent.com'})
    with urlopen(request, timeout=30) as response:
        session = json.load(response)
        if session.get('authenticated') is not False:
            raise ValueError('Anonymous session contract failed')
        if response.headers.get('Access-Control-Allow-Origin') != 'https://app.tryapparent.com':
            raise ValueError('Canonical CORS origin not applied')
        if response.headers.get('Access-Control-Allow-Credentials') != 'true':
            raise ValueError('Credentialed CORS not applied')
    print(json.dumps({'sha': sha, 'deployment': latest['id'], 'version': version['id']}))


if __name__ == '__main__':
    verify(sys.argv[1])
