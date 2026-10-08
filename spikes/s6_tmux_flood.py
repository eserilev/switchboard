#!/usr/bin/env python3
"""S6: can one tmux control client keep a live pane smooth next to 10 panes of `yes`?

The live pane prints a timestamp every 50 ms. We measure the delay from print
to receipt, and the longest gap between two receipts. Noisy panes get paused
by pause-after and are never resumed, the way the app treats tiles.
"""
import os, re, select, subprocess, sys, time

SOCK = "sb-spike"
NOISY = int(sys.argv[1]) if len(sys.argv) > 1 else 10
PAUSE = sys.argv[2] if len(sys.argv) > 2 else "1"   # "0" turns pause-after off, "off" turns noisy output off
SECONDS = 10

def tmux(*a):
    return subprocess.run(["tmux", "-L", SOCK, *a], capture_output=True, text=True)

tmux("kill-server")
tmux("new-session", "-d", "-s", "sb", "-x", "200", "-y", "50",
     "sh", "-c", "while :; do date +%s.%N; sleep 0.05; done")
live = tmux("display", "-p", "-t", "sb:0", "#{pane_id}").stdout.strip()
for _ in range(NOISY):
    tmux("new-window", "-d", "-t", "sb", "yes")

ctl = subprocess.Popen(["tmux", "-L", SOCK, "-C", "attach", "-t", "sb"],
                       stdin=subprocess.PIPE, stdout=subprocess.PIPE, bufsize=0)
def send(cmd): ctl.stdin.write((cmd + "\n").encode())
send("refresh-client -C 200x50")
if PAUSE == "off":
    # Tiles read capture-pane, so only the live pane needs a stream.
    for w in range(1, NOISY + 1):
        pid = tmux("display", "-p", "-t", f"sb:{w}", "#{pane_id}").stdout.strip()
        send(f"refresh-client -A '{pid}:off'")
elif PAUSE != "0":
    send(f"refresh-client -f pause-after={PAUSE}")

buf, lat, gaps, last, nbytes, paused = b"", [], [], None, 0, set()
ts = re.compile(rb"(\d{10}\.\d{9})")
end = time.time() + SECONDS
while time.time() < end:
    r, _, _ = select.select([ctl.stdout], [], [], 0.2)
    if not r: continue
    chunk = os.read(ctl.stdout.fileno(), 1 << 16)
    nbytes += len(chunk); buf += chunk
    *lines, buf = buf.split(b"\n")
    now = time.time()
    for ln in lines:
        if ln.startswith(b"%pause "):
            p = ln.split()[1].decode(); paused.add(p)
            if p == live: send(f"refresh-client -A '{live}:continue'")
        elif ln.startswith((b"%output " + live.encode(), b"%extended-output " + live.encode())):
            for m in ts.findall(ln):
                lat.append(now - float(m))
                if last: gaps.append(now - last)
                last = now

t0 = time.time()
cap = tmux("capture-pane", "-p", "-e", "-S", "-4", "-t", "sb:1")
cap_ms = (time.time() - t0) * 1000
send("kill-server")
ctl.wait(timeout=5)
lat.sort()
print(f"capture-pane of a noisy tile: {cap_ms:.0f}ms, {len(cap.stdout.splitlines())} lines")
print(f"noisy={NOISY} pause-after={PAUSE} bytes={nbytes/1e6:.1f}MB paused={len(paused)}")
if lat:
    print(f"live lines={len(lat)} (expect ~{SECONDS*20}) "
          f"latency p50={lat[len(lat)//2]*1000:.0f}ms p99={lat[int(len(lat)*.99)]*1000:.0f}ms "
          f"max gap={max(gaps)*1000:.0f}ms")
else:
    print("live pane: no lines received")
