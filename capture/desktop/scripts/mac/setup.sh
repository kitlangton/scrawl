#!/bin/bash
# Exec preserves arguments and makes the checked environment reach the child.
set -euo pipefail
exec python3 "$(dirname "$0")/doctor.py" "$@"
