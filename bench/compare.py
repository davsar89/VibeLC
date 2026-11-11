#!/usr/bin/env python3
"""Plays each clip fullscreen to its end in VibeLC and in VLC, both with default settings in a
throwaway configuration, and prints CPU use, dropped frames, memory and the time until the
window appears.

Usage: bench/compare.py [--runs N] CLIP...
Set VIBELC to the vibelc binary to test (default: vibelc on PATH). Needs xdotool and an
otherwise idle desktop."""

import os
import re
import subprocess
import sys
import tempfile
import time

VIBELC = os.environ.get("VIBELC", "vibelc")
# Dropped pictures: mpv's output and decoder counters, and VLC's late-picture warnings.
VIBELC_DROPS = re.compile(r"dropped frames (\d+) \(output\) (\d+) \(decoder\)")
VLC_DROPS = re.compile(r"picture is too late to be displayed|dropping frame")


def system_ticks():
    """(busy, total) jiffies over all CPUs."""
    with open("/proc/stat") as f:
        ticks = [int(v) for v in f.readline().split()[1:9]]
    return sum(ticks) - ticks[3] - ticks[4], sum(ticks)


def play(cmd, env, log_path):
    busy0, total0 = system_ticks()
    start = time.monotonic()
    with open(log_path, "w") as log:
        proc = subprocess.Popen(cmd, env=env, stdout=log, stderr=subprocess.STDOUT)
    search = ["xdotool", "search", "--sync", "--onlyvisible", "--pid", str(proc.pid)]
    subprocess.run(search, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=60)
    window = time.monotonic() - start
    _, _, usage = os.wait4(proc.pid, 0)
    wall = time.monotonic() - start
    busy1, total1 = system_ticks()
    return {
        "cpu": 100 * (usage.ru_utime + usage.ru_stime) / wall,
        "system": 100 * os.cpu_count() * (busy1 - busy0) / (total1 - total0),
        "rss": usage.ru_maxrss / 1024,
        "window": window,
    }


def vibelc(clip, tmp):
    env = dict(os.environ, VIBELC_CONFIG_DIR=f"{tmp}/vibelc")
    log = f"{tmp}/vibelc.log"
    result = play([VIBELC, "--new-window", "--fullscreen", "--quit-at-end", "--stats", clip], env, log)
    counts = VIBELC_DROPS.findall(open(log).read())
    result["dropped"] = sum(map(int, counts[-1])) if counts else None
    return result


def vlc(clip, tmp):
    os.makedirs(f"{tmp}/vlc/config/vlc", exist_ok=True)
    with open(f"{tmp}/vlc/config/vlc/vlcrc", "w") as f:
        f.write("[qt]\nqt-privacy-ask=0\n")  # the first-run dialog only; playback stays default
    env = dict(os.environ, **{f"XDG_{d.upper()}_HOME": f"{tmp}/vlc/{d}" for d in ("config", "cache", "data")})
    log = f"{tmp}/vlc.log"
    result = play(["vlc", "-vv", "--fullscreen", "--play-and-exit", clip], env, log)
    result["dropped"] = len(VLC_DROPS.findall(open(log, errors="replace").read()))
    return result


def main():
    args = sys.argv[1:]
    runs = 1
    if args[:1] == ["--runs"]:
        runs, args = int(args[1]), args[2:]
    if not args:
        sys.exit(__doc__)
    print("clip\tplayer\tCPU %\tsystem CPU %\tdropped\tpeak RSS MB\twindow s")
    for clip in args:
        for _ in range(runs):
            for name, player in (("VibeLC", vibelc), ("VLC", vlc)):
                with tempfile.TemporaryDirectory() as tmp:
                    r = player(os.path.abspath(clip), tmp)
                print(f"{os.path.basename(clip)}\t{name}\t{r['cpu']:.0f}\t{r['system']:.0f}\t{r['dropped']}"
                      f"\t{r['rss']:.0f}\t{r['window']:.2f}", flush=True)


if __name__ == "__main__":
    main()
