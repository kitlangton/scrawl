#!/bin/sh
# Fails if this build could still trust the eMudhra intermediates whose private keys were
# disclosed in October 2026 (https://emudhra.we-have-a-problem.com):
#   - Cargo.lock must have webpki-roots / webpki-root-certs 1.0.10+ (drops "emSign ECC Root CA - G3")
#     and rustls-webpki 0.103.16+ (rejects chains through the eight leaked intermediates, even when
#     the OS trust store still carries their roots).
#   - No tracked file may carry the affected roots as PEM (e.g. a vendored cacert.pem).
#   - Binaries passed as arguments must not embed the affected roots.
#
#   scripts/check-ca-roots.sh [binary...]
set -eu

repo="$(cd "$(dirname "$0")/.." && pwd)"
status=0

if [ -f "$repo/Cargo.lock" ]; then
  stale=$(awk '
    function older(v, min,   a, b, i) {
      split(v, a, "."); split(min, b, ".")
      for (i = 1; i <= 3; i++) if (a[i] + 0 != b[i] + 0) return a[i] + 0 < b[i] + 0
      return 0
    }
    /^name = / { name = $3; gsub(/"/, "", name) }
    /^version = / {
      v = $3; gsub(/"/, "", v)
      # webpki-roots 0.26.11 only re-exports 1.x, so the 1.x entry decides.
      if ((name == "webpki-roots" || name == "webpki-root-certs") && v != "0.26.11" && older(v, "1.0.10"))
        print "  " name " " v " (need 1.0.10+)"
      if (name == "rustls-webpki" && older(v, "0.103.16"))
        print "  " name " " v " (need 0.103.16+)"
    }
  ' "$repo/Cargo.lock")
  if [ -n "$stale" ]; then
    echo "Cargo.lock still trusts the compromised eMudhra CAs:"
    echo "$stale"
    status=1
  fi
fi

# First base64 line of "emSign ECC Root CA - G3" and "AAA Certificate Services".
if git -C "$repo" rev-parse --git-dir >/dev/null 2>&1; then
  pem=$(git -C "$repo" grep -l -F \
    -e MIICTjCCAdOgAwIBAgIKPPYHqWhwDtqLhDAKBggqhkjOPQQDAzBrMQswCQYDVQQG \
    -e MIIEMjCCAxqgAwIBAgIBATANBgkqhkiG9w0BAQUFADB7MQswCQYDVQQGEwJHQjEb \
    -- . ':!scripts/check-ca-roots.sh' || true)
  if [ -n "$pem" ]; then
    echo "Tracked files bundle a compromised root:"
    echo "$pem" | sed 's/^/  /'
    status=1
  fi
else
  echo "skipped PEM scan: $repo is not a git checkout"
fi

for bin in "$@"; do
  if [ ! -f "$bin" ]; then
    echo "$bin: no such file"
    status=1
    continue
  fi
  for cn in "emSign ECC Root CA - G3" "AAA Certificate Services"; do
    if LC_ALL=C grep -q -a -F "$cn" "$bin"; then
      echo "$bin embeds \"$cn\""
      status=1
    fi
  done
done

[ "$status" -eq 0 ] && echo "ok: no compromised eMudhra roots${1:+ in $*}"
exit "$status"
