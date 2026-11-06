#!/bin/sh
set -eu
case "$0" in /*) entry_dir=${0%/*} ;; *) echo 'media entry must be invoked by absolute path' >&2; exit 126 ;; esac
root=$(CDPATH= cd -- "$entry_dir" && pwd -P)
if [ "$#" -eq 8 ]; then
  recovery_nonce=$8
  if [ "$6" != 2 ] || [ "$7" != --recovery-peer-v1 ] || [ "${#recovery_nonce}" -ne 32 ]; then exit 2; fi
  # Character ranges use locale collation on Darwin; nonce bytes are exact ASCII.
  case "$recovery_nonce" in *[!0123456789abcdef]*) exit 2 ;; esac
elif [ "$#" -ne 6 ]; then exit 2; fi
if [ "$1" != --media-runtime-directory ] || [ "$3" != --media-channel ] || [ "$4" != media ] || [ "$5" != --media-protocol-version ] || { [ "$6" != 1 ] && [ "$6" != 2 ]; }; then
  echo 'invalid media launch arguments' >&2
  exit 2
fi
if [ ! -d "$2" ] || [ -L "$2" ]; then echo 'runtime directory must already exist' >&2; exit 2; fi
export LD_LIBRARY_PATH="$root/lib"
export GST_PLUGIN_PATH_1_0="$root/plugins"
export GST_PLUGIN_PATH="$root/plugins"
export GST_PLUGIN_SYSTEM_PATH_1_0=
export GST_PLUGIN_SYSTEM_PATH=
export GST_REGISTRY=/dev/null
export GST_REGISTRY_1_0=/dev/null
export GST_REGISTRY_FORK=no
exec "$root/libexec/apparent-engine-frames" "$@"
