#!/usr/bin/env python3
"""A stand-in for OpenCode's session API, for scripts/e2e.sh.

Lists a few sessions (`GET /api/session?parentID=null…`, newest first) and answers one by ID
(`GET /api/session/<id>`). Prompts never come here: Scrawl posts them through `opencode2`, which
e2e.sh swaps for a script that writes them down.

    python3 fake-opencode.py <port>
"""

import json
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlparse

NOW = int(time.time() * 1000)
SESSIONS = [
    {"id": "ses_e2e_scrawl", "title": "E2E SESSION", "time": {"updated": NOW - 60_000}, "location": {"directory": "/tmp/e2e/scrawl"}},
    {"id": "ses_e2e_child", "title": "A subagent", "parentID": "ses_e2e_scrawl", "time": {"updated": NOW - 30_000}},
    {"id": "ses_e2e_newest", "title": "Newest session", "time": {"updated": NOW - 10_000}, "location": {"directory": "/tmp/e2e/other"}},
    {"id": "ses_e2e_archived", "title": "Archived", "time": {"updated": NOW, "archived": NOW}},
]


class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        url = urlparse(self.path)
        if url.path == "/api/session":
            query = parse_qs(url.query)
            sessions = [s for s in SESSIONS if "parentID" not in s] if query.get("parentID") == ["null"] else SESSIONS
            sessions = sorted(sessions, key=lambda s: -s["time"]["updated"])
            return self.reply(200, {"data": sessions, "cursor": {"next": None}})
        if url.path.startswith("/api/session/"):
            wanted = unquote(url.path.rsplit("/", 1)[1])
            for session in SESSIONS:
                if session["id"] == wanted:
                    return self.reply(200, {"data": session})
        return self.reply(404, {"error": "not found"})

    def reply(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, format, *args):
        sys.stderr.write("fake-opencode: " + format % args + "\n")


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1])), Handler).serve_forever()
