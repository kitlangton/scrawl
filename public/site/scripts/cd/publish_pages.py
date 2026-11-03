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
import sys
from urllib.error import HTTPError
from urllib.request import Request, urlopen

ACCOUNT = 'a54dcebead3b8ad688f891da0ab54d1f'
PROJECT = 'apparent-capture'
ROOT = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(ROOT / 'enablement'))
from release_gate import publication_needed


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
    project = api(f'/accounts/{ACCOUNT}/pages/projects/{PROJECT}')
    deployment = project['canonical_deployment']
    published = None if deployment is None else deployment['deployment_trigger']['metadata']['commit_hash']
    if not publication_needed(os.environ['GITHUB_REPOSITORY'], sha, published):
        raise SupersededMain('An equal or newer website revision is already deployed')


def inventory(root):
    if root.is_symlink() or not root.is_dir():
        raise ValueError('Artifact root must be a regular directory')
    result = {}
    for path in sorted(root.rglob('*')):
        if path.is_symlink() or not (path.is_dir() or path.is_file()):
            raise ValueError('Artifact contains a symlink or nonregular file')
        if path.is_file():
            if path.stat().st_size > 25 * 1024 * 1024:
                raise ValueError('Artifact contains an oversized asset')
            result[path.relative_to(root).as_posix()] = hashlib.sha256(path.read_bytes()).hexdigest()
    return result


def seal(sha):
    if not re.fullmatch('[0-9a-f]{40}', sha):
        raise ValueError('Invalid artifact revision')
    artifact = ROOT / '.build/website'
    # Refuse accidental reuse: an immutable candidate is assembled once.
    artifact.mkdir(parents=True, exist_ok=False)
    shutil.copytree(ROOT / 'public/site/dist-apparent', artifact / 'dist-apparent')
    shutil.copyfile(ROOT / 'public/site/wrangler.pages.jsonc', artifact / 'wrangler.jsonc')
    files = inventory(artifact)
    for required in ('dist-apparent/index.html', 'dist-apparent/404.html',
                     'dist-apparent/_routes.json', 'dist-apparent/_worker.js'):
        if required not in files:
            raise ValueError('Incomplete Pages artifact: ' + required)
    movie = ROOT / 'public/site/public/assets/media/Apparent-ghost-demo-v4-lilac.mp4'
    media = {'path': '/' + movie.name, 'sha256': hashlib.sha256(movie.read_bytes()).hexdigest()}
    manifest = {'schema': 1, 'revision': sha, 'files': files, 'movie': media}
    (artifact / 'manifest.json').write_text(json.dumps(manifest, sort_keys=True, indent=2) + '\n')
    verify_artifact(sha, artifact)
    return manifest


def verify_artifact(sha, artifact):
    actual = inventory(artifact)
    manifest = json.loads((artifact / 'manifest.json').read_text())
    actual.pop('manifest.json')
    if manifest['schema'] != 1 or manifest['revision'] != sha:
        raise ValueError('Artifact revision mismatch')
    if actual != manifest['files']:
        raise ValueError('Artifact checksum/inventory mismatch')
    if any(not (name == 'wrangler.jsonc' or name.startswith('dist-apparent/')) for name in actual):
        raise ValueError('Unexpected artifact input')
    return manifest


class CanonicalPropagationPending(ValueError):
    """Immutable deployment passed, but the public hostname still differs."""


class ReadbackTransferIncomplete(OSError):
    """The HTTP body ended before its advertised Content-Length."""


def verify(sha, package):
    project = api(f'/accounts/{ACCOUNT}/pages/projects/{PROJECT}')
    deployment = project['canonical_deployment']
    if deployment['latest_stage']['status'] != 'success' or deployment['deployment_trigger']['metadata']['commit_hash'] != sha:
        raise ValueError('Pages production is not the checked revision')
    manifest = verify_artifact(sha, package.parent)
    paths = [('index.html', '/'), ('connect/index.html', '/connect/'),
             ('connect/device/index.html', '/connect/device/'),
             ('connect/shares/index.html', '/connect/shares/')]
    # Check emitted code as well as HTML, at both immutable and public origins.
    paths += [(path.removeprefix('dist-apparent/'), '/' + path.removeprefix('dist-apparent/'))
              for path in manifest['files'] if path.startswith('dist-apparent/assets/') and path.endswith(('.js', '.css'))]
    for origin_index, origin in enumerate(dict.fromkeys([deployment['url'], 'https://app.tryapparent.com'])):
        mismatch = CanonicalPropagationPending if origin_index else ValueError
        for path, route in paths:
            request = Request(origin + route, headers={'Cache-Control': 'no-cache', 'User-Agent': 'Apparent-CD/1.0'})
            with urlopen(request, timeout=60) as response:
                body = response.read()
                response_meta = {
                    'content_encoding': response.headers.get('Content-Encoding'),
                    'content_type': response.headers.get('Content-Type'),
                    'cache_status': response.headers.get('CF-Cache-Status'),
                    'age': response.headers.get('Age'),
                    'cf_ray': response.headers.get('CF-Ray'),
                }
                if response.headers.get('X-Apparent-Publisher') != 'cloudflare-pages':
                    raise ValueError('Missing Pages publisher header')
                if route.startswith('/connect/') and (
                        'no-store' not in response.headers.get('Cache-Control', '').lower() or
                        not {'noindex', 'nofollow'} <= set(response.headers.get('X-Robots-Tag', '').lower().replace(',', ' ').split())):
                    raise ValueError('Missing Connect privacy headers')
            actual_hash = hashlib.sha256(body).hexdigest()
            expected_hash = manifest['files']['dist-apparent/' + path]
            if actual_hash != expected_hash:
                # Public response metadata and fingerprints only: never log the
                # body, cookies, request credentials or complete header sets.
                detail = {**response_meta, 'actual_sha256': actual_hash,
                          'expected_sha256': expected_hash, 'actual_bytes': len(body),
                          'expected_bytes': (package / path).stat().st_size}
                raise mismatch(f'Pages served different bytes: {origin}{route} ' + json.dumps(detail, sort_keys=True))
        media = manifest['movie']
        actual = hashlib.sha256()
        actual_bytes = 0
        request = Request(origin + media['path'], headers={'Cache-Control': 'no-cache', 'User-Agent': 'Apparent-CD/1.0'})
        with urlopen(request, timeout=60) as response:
            response_meta = {
                'status': getattr(response, 'status', None),
                'content_length': response.headers.get('Content-Length'),
                'content_range': response.headers.get('Content-Range'),
                'content_encoding': response.headers.get('Content-Encoding'),
                'cache_status': response.headers.get('CF-Cache-Status'),
                'age': response.headers.get('Age'),
                'cf_ray': response.headers.get('CF-Ray'),
            }
            if response.headers.get('Content-Type') != 'video/mp4':
                raise ValueError('Pages movie is unavailable')
            while data := response.read(256 * 1024):
                actual.update(data)
                actual_bytes += len(data)
        detail = {**response_meta, 'actual_sha256': actual.hexdigest(),
                  'expected_sha256': media['sha256'], 'actual_bytes': actual_bytes}
        length = response_meta['content_length']
        if length is not None and actual_bytes != int(length):
            raise ReadbackTransferIncomplete(f'Pages movie transfer incomplete: {origin}{media["path"]} ' + json.dumps(detail, sort_keys=True))
        if actual.hexdigest() != media['sha256']:
            raise mismatch(f'Pages served different movie bytes: {origin}{media["path"]} ' + json.dumps(detail, sort_keys=True))
    return {'revision': sha, 'project': PROJECT, 'deployment_id': deployment['id'],
            'url': deployment['url'], 'public_url': 'https://app.tryapparent.com',
            'manifest_sha256': hashlib.sha256((package.parent / 'manifest.json').read_bytes()).hexdigest()}


def verify_ready(sha, package):
    # A successful deployment can precede Function/asset propagation. Retry
    # temporary HTTP/short-transfer failures and canonical propagation after the immutable
    # deployment passes. Identity, immutable integrity and privacy failures fail
    # immediately; canonical bytes must still match exactly before success.
    for attempt in range(5):
        try:
            return verify(sha, package)
        except (CanonicalPropagationPending, ReadbackTransferIncomplete):
            if attempt == 4:
                raise
            time.sleep(2 ** attempt)
        except HTTPError as error:
            if error.code not in (404, 500, 502, 503, 504) or attempt == 4:
                raise
            error.close()
            time.sleep(2 ** attempt)


def main():
    sha = os.environ['GITHUB_SHA']
    if not re.fullmatch('[0-9a-f]{40}', sha) or os.environ.get('GITHUB_REF') != 'refs/heads/main':
        raise ValueError('Only exact main revisions may publish')
    artifact = ROOT / '.build/website'
    manifest = verify_artifact(sha, artifact)
    package = artifact / 'dist-apparent'
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
        shutil.copyfile(artifact / 'wrangler.jsonc', stage / 'wrangler.jsonc')
        # Verify the actual staged bytes before giving them to the publisher.
        if inventory(stage) != manifest['files']:
            raise ValueError('Staged artifact checksum mismatch')
        try:
            current_main(sha)
        except SupersededMain:
            print('Skipping superseded main candidate; no website published')
            return
        subprocess.run([str(ROOT / 'connect/service/node_modules/.bin/wrangler'), 'pages', 'deploy',
                        '--project-name', PROJECT, '--branch', 'main', '--commit-hash', sha,
                        '--commit-dirty=false', '--no-bundle'], cwd=stage, env=env, check=True)
    receipt = verify_ready(sha, package)
    (ROOT / 'website-receipt.json').write_text(json.dumps(receipt) + '\n')
    print(json.dumps(receipt))


if __name__ == '__main__':
    if len(sys.argv) == 3 and sys.argv[1] == 'seal':
        seal(sys.argv[2])
    elif len(sys.argv) == 1:
        main()
    else:
        raise ValueError('Usage: publish_pages.py [seal SHA]')
