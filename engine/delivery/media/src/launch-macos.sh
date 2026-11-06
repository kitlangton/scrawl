#!/bin/sh
set -eu
case "$0" in /*) entry_dir=${0%/*} ;; *) echo 'media entry must be absolute' >&2; exit 126 ;; esac
root=$(CDPATH= cd -- "$entry_dir" && pwd -P)
if [ "$#" -ne 6 ] || [ "$1" != --media-runtime-directory ] || [ "$3" != --media-channel ] || [ "$4" != media ] || [ "$5" != --media-protocol-version ] || [ "$6" != 1 ]; then
  echo 'invalid media launch arguments' >&2
  exit 2
fi
unset DYLD_LIBRARY_PATH DYLD_FALLBACK_LIBRARY_PATH DYLD_FRAMEWORK_PATH DYLD_FALLBACK_FRAMEWORK_PATH DYLD_INSERT_LIBRARIES
exec "$root/libexec/apparent-engine-frames" "$@"
