#!/bin/sh
set -eu
case "$0" in /*) entry_dir=${0%/*} ;; *) echo 'capture entry must be invoked by absolute path' >&2; exit 126 ;; esac
root=$(CDPATH= cd -- "$entry_dir" && pwd -P)
if [ "$#" -ne 6 ] || [ "$1" != --runtime-directory ] || [ "$3" != --capture-channel ] || [ "$4" != capture ] || [ "$5" != --capture-protocol-version ] || [ "$6" != 2 ]; then
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
exec "$root/libexec/apparent-engine-capture" "$@"
