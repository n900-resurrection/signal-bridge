#!/usr/bin/env python3
"""Tiny command line client of signal-bridge, for tests: sb-client.py [--socket PATH] [--wait SECONDS] [LINE ...]
Each LINE is sent as one protocol line, written with \\t between the fields (SEND\\tid\\tUUID\\ttext). Everything the bridge
sends is printed until --wait seconds pass without news. READY is shown without the identity."""
import os
import socket
import sys
import time

args = sys.argv[1:]
path = os.path.expanduser("~/.local/share/signal-bridge/bridge.sock")
wait = 15.0
lines = []
while args:
    a = args.pop(0)
    if a == "--socket":
        path = args.pop(0)
    elif a == "--wait":
        wait = float(args.pop(0))
    else:
        lines.append(a.replace("\\t", "\t"))

s = socket.socket(socket.AF_UNIX)
s.settimeout(wait)
s.connect(path)
f = s.makefile("rwb", buffering=0)
for l in lines:
    f.write((l + "\n").encode())
    time.sleep(0.2)
try:
    for raw in f:
        text = raw.decode("utf-8", "replace").rstrip("\n")
        if text.startswith("READY"):
            text = "READY <identity hidden>"
        print(text.replace("\t", " | "), flush=True)
except (socket.timeout, TimeoutError):
    pass
