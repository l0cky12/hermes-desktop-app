"""Test-only XDND drag source: a small GTK window whose button drags the given files.

    python3 mock/drag.py FILE...   then drive the drag with xdotool (see README).
"""
import sys
from pathlib import Path

import gi

gi.require_version("Gdk", "3.0")
gi.require_version("Gtk", "3.0")
from gi.repository import Gdk, Gtk  # noqa: E402

uris = [Path(p).resolve().as_uri() for p in sys.argv[1:]]
window = Gtk.Window(title="drag-source")
window.move(1180, 20)
button = Gtk.Button(label="drag files")
button.set_size_request(180, 120)
button.drag_source_set(Gdk.ModifierType.BUTTON1_MASK, [], Gdk.DragAction.COPY)
button.drag_source_add_uri_targets()
button.connect("drag-data-get", lambda _w, _ctx, data, _info, _time: data.set_uris(uris))
window.add(button)
window.connect("destroy", Gtk.main_quit)
window.show_all()
Gtk.main()
