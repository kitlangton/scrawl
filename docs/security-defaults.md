# Capture delivery security defaults

Automatic OpenCode delivery is opt-in. Enable **Send dev-site captures to OpenCode**
in Settings or set `"autoSend": true` in the config to enable automatic forwarding.
The same preference applies to HTTP and USB imports; `SCRAWL_NO_OPENCODE=1`
continues to override it. Explicit `scrawl send` remains available.

The ingest HTTP server binds to `127.0.0.1`, rather than all interfaces. Direct
plain-HTTP uploads from another device no longer reach the server. For remote
ScrawlKit uploads, use an authenticated HTTPS proxy or encrypted tunnel to the
loopback service and update the device pairing to the appropriate endpoint.
Do not expose the HTTP backend directly to an untrusted network. Existing LAN
pairing URLs need to be replaced. Local SDK registrations continue to work.

The server permits eight concurrent connections. Headers must finish within five
seconds and bodies within thirty seconds, even if the client continues sending
small amounts of data. Header size and existing per-route body limits still apply.
`SCRAWL_NO_INGEST=1` disables the GUI app's HTTP listener and USB import poller.

Opening a ScrawlKit pairing link now displays an approval alert before changing
or clearing credentials. Without an available foreground presenter, the pairing
is left unchanged. Queued recordings are bound to their original upload URL:
switching the pairing to another endpoint does not transfer queued recordings
to it. Renewing credentials for the same URL can resume those uploads.

Captures may retain source audio, video, and uncropped frames. Cropping is a
presentation feature, not redaction. Review a bundle before deliberately sharing
it. Retention moves data to Trash/Recycle Bin and does not securely erase it.

Windows release ZIPs remain unsigned. Their SHA-256 files are now attached to the
GitHub release alongside the ZIP. Checksums do not replace publisher signatures.
