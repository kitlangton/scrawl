#!/usr/bin/env python3
"""Own a restored Wayland selection with all its original MIME types.

Private binary protocol on stdin: u32 item count, then u32 MIME length, MIME,
u32 data length, data per item. Nothing is written to disk or stdout except READY.
"""
import struct
import sys

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gdk, GLib, Gtk


def read_exact(size):
    data = sys.stdin.buffer.read(size)
    if len(data) != size:
        raise ValueError("Incomplete clipboard snapshot")
    return data


def number():
    return struct.unpack("!I", read_exact(4))[0]


def main():
    count = number()
    if not 1 <= count <= 32:
        raise ValueError("Invalid clipboard MIME count")
    providers = []
    total = 0
    for _ in range(count):
        mime_size = number()
        if not 1 <= mime_size <= 256:
            raise ValueError("Invalid clipboard MIME size")
        mime = read_exact(mime_size).decode("utf-8")
        size = number()
        total += size
        if total > 16 * 1024 * 1024:
            raise ValueError("Clipboard snapshot is too large")
        providers.append(Gdk.ContentProvider.new_for_bytes(mime, GLib.Bytes.new(read_exact(size))))
    Gtk.init()
    display = Gdk.Display.get_default()
    if display is None:
        raise RuntimeError("No Wayland display")
    clipboard = display.get_clipboard()
    provider = Gdk.ContentProvider.new_union(providers)
    if not clipboard.set_content(provider):
        raise RuntimeError("Could not restore clipboard")
    loop = GLib.MainLoop()
    clipboard.connect("changed", lambda cb: loop.quit() if cb.get_content() != provider else None)
    sys.stdout.write("READY\n")
    sys.stdout.flush()
    sys.stdout.close()
    loop.run()


if __name__ == "__main__":
    if sys.argv[1:] == ["--check"]:
        Gtk.init()
        if Gdk.Display.get_default() is None:
            raise RuntimeError("No Wayland display for clipboard restoration")
        print("READY")
    else:
        main()
