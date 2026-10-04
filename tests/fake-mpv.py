#!/usr/bin/env python3
"""A stand-in for mpv in the player tests: serves the JSON IPC socket the
way mpv does, reports a position and a length, and ends the way
--fake-end= says: "eof" plays to the end, "quit" waits for anchor's quit
command, "error" fails like a refused debrid link, "early" exits before
opening the socket. --fake-args= names a file the arguments are written
to, one per line. Both come in as the user's extra arguments."""

import json
import os
import socket
import sys
import time

args = sys.argv[1:]


def option(name, default=None):
    return next((a.split("=", 1)[1] for a in args if a.startswith(name + "=")), default)


if option("--fake-args"):
    with open(option("--fake-args"), "w") as f:
        f.write("\n".join(args) + "\n")
end = option("--fake-end", "eof")
if end == "early":
    print("Error parsing option fs (option not found)", file=sys.stderr)
    sys.exit(1)
path = next(a.split("=", 1)[1] for a in args if a.startswith("--input-ipc-server="))
start = next((float(a.split("=", 1)[1]) for a in args if a.startswith("--start=")), 0.0)

server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(path)
server.listen(1)
conn, _ = server.accept()
reader = conn.makefile("r")
observed = {}
for _ in range(3):
    command = json.loads(reader.readline())["command"]
    observed[command[2]] = command[1]
    conn.sendall(b'{"request_id":0,"error":"success"}\n')


def send(message):
    conn.sendall((json.dumps(message) + "\n").encode())


def change(name, value):
    send({"event": "property-change", "id": observed[name], "name": name, "data": value})


if end == "error":
    print("[ffmpeg] https: HTTP error 403 Forbidden https://debrid.example/key", file=sys.stderr)
    send({"event": "end-file", "reason": "error", "file_error": "loading failed"})
    conn.close()
    sys.exit(2)

change("duration", 100.0)
change("pause", False)
for step in range(5):
    change("time-pos", start + step)
    time.sleep(0.12)
if end == "quit":
    conn.settimeout(10)
    while json.loads(reader.readline())["command"] != ["quit"]:
        pass
    send({"event": "end-file", "reason": "quit"})
else:
    change("time-pos", 100.0)
    send({"event": "end-file", "reason": "eof"})
conn.close()
os.unlink(path)
