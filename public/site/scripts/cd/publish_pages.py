"""Publish only the checked main package to the capture site's Pages project."""
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import shutil
import tempfile
import time
from urllib.error import HTTPError
from urllib.request import Request, urlopen

ACCOUNT = 'a54dcebead3b8ad688f891da0ab54d1f'
PROJECT = 'apparent-capture'
ROOT = Path(__file__).resolve().parents[4]


def api(path):
    request = Request('https://api.cloudflare.com/client/v4' + path,
                      headers={'Authorization': 'Bearer ' + os.environ['WEBSITE_PAGES_API_TOKEN']})
    with urlopen(request, timeout=60) as response:
        value = json.load(response)
    if not value.get('success'):
        raise ValueError('Pages readback failed')
    return value['result']


class SupersededMain(ValueError):
    pass


def current_main(sha):
    tip = subprocess.check_output(['git', 'ls-remote', 'origin', 'refs/heads/main'], cwd=ROOT, text=True).split()[0]
    if tip != sha:
        raise SupersededMain('Refusing stale main revision')


def verify(sha, package):
    project = api(f'/accounts/{ACCOUNT}/pages/projects/{PROJECT}')
    deployment = project['canonical_deployment']
    if deployment['latest_stage']['status'] != 'success' or deployment['deployment_trigger']['metadata']['commit_hash'] != sha:
        raise ValueError('Pages production is not the checked revision')
    origin = deployment['url']
    for path, route in [('index.html', '/'), ('connect/index.html', '/connect/'),
                        ('connect/device/index.html', '/connect/device/'),
                        ('connect/shares/index.html', '/connect/shares/')]:
        request = Request(origin + route, headers={'Cache-Control': 'no-cache', 'User-Agent': 'Apparent-CD/1.0'})
        with urlopen(request, timeout=60) as response:
            body = response.read()
            if response.headers.get('X-Apparent-Publisher') != 'cloudflare-pages':
                raise ValueError('Missing Pages publisher header')
            if route.startswith('/connect/') and (
                    'no-store' not in response.headers.get('Cache-Control', '').lower() or
                    not {'noindex', 'nofollow'} <= set(response.headers.get('X-Robots-Tag', '').lower().replace(',', ' ').split())):
                raise ValueError('Missing Connect privacy headers')
        if hashlib.sha256(body).digest() != hashlib.sha256((package / path).read_bytes()).digest():
            raise ValueError('Pages served different bytes: ' + path)
    movie = ROOT / 'public/site/public/assets/media/Apparent-ghost-demo-v4-lilac.mp4'
    expected = hashlib.sha256(movie.read_bytes()).digest()
    actual = hashlib.sha256()
    request = Request(origin + '/' + movie.name, headers={'Cache-Control': 'no-cache', 'User-Agent': 'Apparent-CD/1.0'})
    with urlopen(request, timeout=60) as response:
        if response.headers.get('Content-Type') != 'video/mp4':
            raise ValueError('Pages movie is unavailable')
        while data := response.read(256 * 1024):
            actual.update(data)
    if actual.digest() != expected:
        raise ValueError('Pages served different movie bytes')
    return {'revision': sha, 'project': PROJECT, 'deployment_id': deployment['id'], 'url': origin}


def verify_ready(sha, package):
    # A successful deployment can precede Function/asset propagation. Retry
    # only temporary HTTP readiness failures, never identity or hash failures.
    for attempt in range(5):
        try:
            return verify(sha, package)
        except HTTPError as error:
            if error.code not in (404, 502, 503, 504) or attempt == 4:
                raise
            error.close()
            time.sleep(2 ** attempt)


def main():
    sha = os.environ['GITHUB_SHA']
    if not re.fullmatch('[0-9a-f]{40}', sha) or os.environ.get('GITHUB_REF') != 'refs/heads/main':
        raise ValueError('Only exact main revisions may publish')
    package = ROOT / 'public' / 'site' / 'dist-apparent'
    if not (package / '404.html').is_file() or not (package / '_routes.json').is_file():
        raise ValueError('Missing privacy/routing preparation')
    if package.is_symlink() or any(p.is_symlink() or (p.is_file() and p.stat().st_size > 25 * 1024 * 1024) for p in package.rglob('*')):
        raise ValueError('Package has a symlink or oversized asset')
    env = {**os.environ, 'CLOUDFLARE_API_TOKEN': os.environ['WEBSITE_PAGES_API_TOKEN'],
           'CLOUDFLARE_ACCOUNT_ID': ACCOUNT, 'WRANGLER_SEND_METRICS': 'false'}
    # Pages CLI discovers only the standard config filename. Isolate it from the
    # unrelated Worker config, and stage no repository/private files.
    with tempfile.TemporaryDirectory(prefix='apparent-pages-') as directory:
        stage = Path(directory)
        shutil.copytree(package, stage / 'dist-apparent')
        shutil.copytree(ROOT / 'public/site/functions', stage / 'functions')
        shutil.copytree(ROOT / 'public/site/lib', stage / 'lib')
        shutil.copyfile(ROOT / 'public/site/wrangler.pages.jsonc', stage / 'wrangler.jsonc')
        try:
            current_main(sha)
        except SupersededMain:
            print('Skipping superseded main candidate; no website published')
            return
        subprocess.run([str(ROOT / 'connect/service/node_modules/.bin/wrangler'), 'pages', 'deploy',
                        '--project-name', PROJECT, '--branch', 'main', '--commit-hash', sha,
                        '--commit-dirty=false'], cwd=stage, env=env, check=True)
    receipt = verify_ready(sha, package)
    (ROOT / 'website-receipt.json').write_text(json.dumps(receipt) + '\n')
    print(json.dumps(receipt))


if __name__ == '__main__':
    main()
