#!/usr/bin/env python3
"""Guest-only HTTPS release fixture. Never run as a host GitHub proxy.

The production updater, URLs, curl TLS verification and checksum/atomic install
path are unchanged. A guest-local CONNECT proxy terminates only the two names.
"""
import argparse
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import ssl
import time

API = '/repos/tryapparent/apparent/releases/latest'
MAIN_API = '/repos/tryapparent/apparent/releases?per_page=10&page='
CANDIDATE_SHA = 'b' * 40


def metadata_path(control):
    return MAIN_API + '1' if control.get('channel', 'stable') == 'main' else API


def response(control, path, payload):
    mode = control['mode']
    version = control['version'] if mode == 'current' else '999.0.0'
    main = control.get('channel', 'stable') == 'main'
    revision = control['source_head'] if main and mode == 'current' else CANDIDATE_SHA
    name = f'Apparent-{version}-linux-x86_64.AppImage'
    tag = 'linux-main-' + revision if main else 'v' + version
    prefix = f'/tryapparent/apparent/releases/download/{tag}/'
    release = {'tag_name': tag, 'draft': False, 'prerelease': main,
               'published_at': '2026-11-01T00:00:00Z',
               'assets': [{'name': filename, 'browser_download_url': 'https://github.com' + prefix + filename}
                          for filename in [name, name + '.sha256']]}
    if path == metadata_path(control):
        if mode == 'error':
            return 503, b'controlled fixture failure\n'
        # Put the usable Main release on a later page, beyond unrelated releases.
        return 200, json.dumps([{'tag_name': 'v1.0.0', 'prerelease': False,
                                'draft': False, 'assets': []}] if main else release).encode()
    if main and path == MAIN_API + '2':
        return 200, json.dumps([release]).encode()
    if main and path == MAIN_API + '3':
        return 200, b'[]'
    if main:
        compare = '/repos/tryapparent/apparent/compare/'
        source = control['source_head']
        query = '?per_page=1&page=2'
        if path == compare + source + '...' + CANDIDATE_SHA + query:
            return 200, json.dumps({'status': 'diverged' if mode == 'diverged' else 'ahead',
                                    'base_commit': {'sha': source}}).encode()
        if path == compare + CANDIDATE_SHA + '...' + source + query:
            return 200, json.dumps({'status': 'behind', 'base_commit': {'sha': CANDIDATE_SHA}}).encode()
    if path == prefix + name + '.sha256':
        digest = '0' * 64 if mode == 'bad-checksum' else hashlib.sha256(payload).hexdigest()
        return 200, f'{digest}  {name}\n'.encode()
    if path == prefix + name:
        return 200, payload
    return 404, b'unexpected fixture route\n'


def serve(directory, payload, port=8443):
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.load_cert_chain(directory / 'cert.pem', directory / 'key.pem')

    class Handler(BaseHTTPRequestHandler):
        def do_CONNECT(self):
            if self.path not in ('api.github.com:443', 'github.com:443'):
                self.send_error(403, 'fixture does not proxy other destinations')
                return
            self.send_response(200, 'Connection Established')
            self.end_headers()
            self.connection = context.wrap_socket(self.connection, server_side=True)
            self.rfile = self.connection.makefile('rb')
            self.wfile = self.connection.makefile('wb')
            self.close_connection = True
            self.handle_one_request()

        def do_GET(self):
            control = json.loads((directory / 'control.json').read_text())
            with (directory / 'requests.jsonl').open('a') as log:
                log.write(json.dumps({'time': time.time(), 'path': self.path, 'control': control}) + '\n')
            if self.path == metadata_path(control) and control.get('gate'):
                (directory / 'request-started').touch()
                deadline = time.monotonic() + 30
                while not (directory / 'release-response').exists():
                    if time.monotonic() > deadline:
                        self.send_error(504, 'fixture gate was not released')
                        return
                    time.sleep(0.05)
            status, body = response(control, self.path, payload)
            self.send_response(status)
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            if control['serial'] == 1 and self.path.endswith('.sha256'):
                (directory / 'startup-complete').touch()

        def log_message(self, format, *args):
            pass

    server = ThreadingHTTPServer(('127.0.0.1', port), Handler)
    (directory / 'server-ready').touch()
    server.serve_forever()


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('appimage', type=Path)
    parser.add_argument('--port', type=int, default=8443)
    args = parser.parse_args()
    serve(args.directory, args.appimage.read_bytes(), args.port)
