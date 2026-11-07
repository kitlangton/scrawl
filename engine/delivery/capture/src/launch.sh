#!/bin/sh
set -eu
case "$0" in /*) entry_dir=${0%/*} ;; *) echo 'capture entry must be invoked by absolute path' >&2; exit 126 ;; esac
root=$(CDPATH= cd -- "$entry_dir" && pwd -P)
if [ "$#" -eq 8 ]; then
  recovery_nonce=$8
  if [ "$7" != --recovery-peer-v1 ] || [ "${#recovery_nonce}" -ne 32 ]; then exit 2; fi
  # Character ranges use locale collation on Darwin; nonce bytes are exact ASCII.
  case "$recovery_nonce" in *[!0123456789abcdef]*) exit 2 ;; esac
elif [ "$#" -ne 6 ]; then exit 2; fi
if [ "$1" != --runtime-directory ] || [ "$3" != --capture-channel ] || [ "$4" != capture ] || [ "$5" != --capture-protocol-version ] || [ "$6" != 2 ]; then
  echo 'invalid capture launch arguments' >&2
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
export PIPEWIRE_MODULE_DIR="$root/pipewire"
export SPA_PLUGIN_DIR="$root/spa"
export PIPEWIRE_CONFIG_DIR="$root/pipewire-config"
export PIPEWIRE_CONFIG_NAME=client.conf
export PIPEWIRE_CONFIG_PREFIX=
unset PIPEWIRE_NO_CONFIG
exec "$root/libexec/apparent-engine-capture" "$@"
