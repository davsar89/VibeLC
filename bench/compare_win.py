#!/usr/bin/env python3
"""Windows counterpart of compare.py: plays each clip fullscreen to its end in VibeLC and in VLC,
both with default settings (VibeLC in a throwaway configuration folder, VLC with its
configuration ignored), and prints CPU use, the GPU engines' use, dropped frames, peak memory
the time until the window appears and, for VibeLC, until the first video frame. CPU is measured
from 3 s after the window appears to the end (steady playback), for the player and for dwm.exe,
which composes and presents the frames. --sw makes both players decode on the processor.
--startup N first opens and closes each player N times without a file. --threads also splits the
steady CPU by thread name (as the threads named themselves; "ui" is the window's thread, which
also renders), in percent of one core, so the lines add up to the process's CPU.

Usage: python bench/compare_win.py [--runs N] [--startup N] [--sw] [--threads] [--only-vibelc] [--vibelc-args "..."] CLIP...
Set VIBELC to the vibelc.exe to test (default: target/release/vibelc.exe; libmpv-2.dll next to
it or VIBELC_LIBMPV set) and VLC to vlc.exe (default: the
standard install). Needs psutil (pip install psutil) and an otherwise idle desktop."""

import csv
import ctypes
import ctypes.wintypes as wt
import os
import re
import shlex
import subprocess
import sys
import tempfile
import time

import psutil

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
VIBELC = os.environ.get("VIBELC", os.path.join(ROOT, "target", "release", "vibelc.exe"))
VLC = os.environ.get("VLC", r"C:\Program Files\VideoLAN\VLC\vlc.exe")
VLC_BASE = [VLC, "--ignore-config", "--no-qt-privacy-ask", "--no-qt-updates-notif", "--no-qt-recentplay"]
# Dropped pictures: mpv's output and decoder counters, and VLC's late-picture warnings.
VIBELC_DROPS = re.compile(r"dropped frames (\d+) \(output\) (\d+) \(decoder\)")
VIBELC_HWDEC = re.compile(r"hwdec (\S+)")
VLC_DROPS = re.compile(r"picture is too late to be displayed|dropping frame")
FIRST_FRAME = re.compile(r"^\[\s*([\d.]+)\] INFO\s+\S+: first video frame", re.M)
STEADY_AFTER = 3.0  # seconds after the window appears
THREADS = False  # --threads

user32 = ctypes.windll.user32
kernel32 = ctypes.windll.kernel32
kernel32.OpenThread.restype = wt.HANDLE
kernel32.GetThreadDescription.argtypes = [wt.HANDLE, ctypes.POINTER(wt.LPWSTR)]
ENUM_PROC = ctypes.WINFUNCTYPE(wt.BOOL, wt.HWND, wt.LPARAM)


def has_visible_window(pid):
    """The process's top-level window larger than 100x100, if it shows one."""
    found = []

    def check(hwnd, _):
        owner = wt.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(hwnd):
            r = wt.RECT()
            user32.GetWindowRect(hwnd, ctypes.byref(r))
            if r.right - r.left > 100 and r.bottom - r.top > 100:
                found.append(hwnd)
                return False
        return True

    user32.EnumWindows(ENUM_PROC(check), 0)
    return found[0] if found else None


def modules(process):
    """[(start, end, name)] of the DLLs loaded in the process."""
    psapi, handles, needed = ctypes.windll.psapi, (wt.HMODULE * 1024)(), wt.DWORD()
    psapi.EnumProcessModulesEx(int(process), handles, ctypes.sizeof(handles), ctypes.byref(needed), 3)
    found = []
    for h in handles[: needed.value // ctypes.sizeof(wt.HMODULE)]:
        info, name = (ctypes.c_void_p * 3)(), ctypes.create_unicode_buffer(260)  # MODULEINFO
        psapi.GetModuleInformation(int(process), wt.HMODULE(h), info, ctypes.sizeof(info))
        psapi.GetModuleBaseNameW(int(process), wt.HMODULE(h), name, 260)
        found.append((info[0], info[0] + info[1], name.value))
    return found


def thread_times(ps, ui_tid, dlls):
    """{thread id: (name, CPU seconds)} for the process's live threads; an unnamed thread is named
    after the DLL its start address lies in."""
    times = {}
    for t in ps.threads():
        name, handle = "(unnamed)", kernel32.OpenThread(0x0840, False, t.id)  # QUERY_(LIMITED_)INFORMATION
        if handle:
            text, start = wt.LPWSTR(), ctypes.c_void_p()
            if kernel32.GetThreadDescription(handle, ctypes.byref(text)) >= 0 and text.value:
                name = text.value
            # ThreadQuerySetWin32StartAddress
            elif ctypes.windll.ntdll.NtQueryInformationThread(handle, 9, ctypes.byref(start), 8, None) == 0:
                name = next((f"({n})" for a, b, n in dlls if start.value and a <= start.value < b), name)
            kernel32.LocalFree(text)
            kernel32.CloseHandle(handle)
        times[t.id] = ("ui" if t.id == ui_tid else name, t.user_time + t.system_time)
    return times


def cpu_seconds(handle):
    """User plus kernel time of a process, also after it has exited (its handle stays open)."""
    times = [wt.FILETIME() for _ in range(4)]
    kernel32.GetProcessTimes(int(handle), *(ctypes.byref(t) for t in times))
    seconds = lambda f: ((f.dwHighDateTime << 32) | f.dwLowDateTime) / 1e7
    return seconds(times[2]) + seconds(times[3])


class GpuSampler:
    """Samples the process's GPU engines once a second with typeperf. Its counters only exist once
    the process uses the GPU, so it starts after the window appears."""

    def __init__(self, pid, path):
        self.path = path
        self.proc = subprocess.Popen(
            ["typeperf", rf"\GPU Engine(pid_{pid}_*)\Utilization Percentage", "-si", "1", "-f", "CSV", "-o", path, "-y"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

    def stop(self):
        """Mean use per engine type in percent, summed over engines of that type."""
        self.proc.terminate()
        self.proc.wait()
        try:
            rows = list(csv.reader(open(self.path, newline="", errors="replace")))
        except FileNotFoundError:
            return {}
        if len(rows) < 3:
            return {}
        # The first sample is often garbage and the last one cut short.
        header, data = rows[0], rows[2:-1] or rows[1:]
        by_type = {}
        for i, name in enumerate(header[1:], 1):
            m = re.search(r"engtype_([A-Za-z0-9 ]+)\)", name)
            if not m:
                continue
            values = []
            for row in data:
                try:
                    values.append(min(100.0, float(row[i])))
                except (ValueError, IndexError):
                    pass
            if values:
                by_type.setdefault(m.group(1), []).append(sum(values) / len(values))
        return {t: sum(v) for t, v in by_type.items()}


def dwm_seconds():
    times = [p.info["cpu_times"] for p in psutil.process_iter(["name", "cpu_times"]) if p.info["name"] == "dwm.exe"]
    return sum(t.user + t.system for t in times if t)


def play(cmd, env, tmp, timeout=300, close_after=None):
    """Runs a player until it exits (or `close_after` seconds after its window appears)."""
    start = time.perf_counter()
    proc = subprocess.Popen(cmd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    ps = psutil.Process(proc.pid)
    window, peak, gpu, steady = None, 0, None, None
    ui_tid, first_threads, last_threads = None, {}, {}
    while proc.poll() is None:
        now = time.perf_counter() - start
        if window is None and (hwnd := has_visible_window(proc.pid)):
            window = now
            ui_tid = user32.GetWindowThreadProcessId(hwnd, None)
            if close_after is None:
                time.sleep(1.5)
                gpu = GpuSampler(proc.pid, os.path.join(tmp, "gpu.csv"))
        if steady is None and window is not None and close_after is None and now >= window + STEADY_AFTER:
            steady = (time.perf_counter(), cpu_seconds(proc._handle), dwm_seconds())
            dlls = modules(proc._handle) if THREADS else []
            first_threads = thread_times(ps, ui_tid, dlls) if THREADS else {}
        if THREADS and steady:
            # Threads are gone once the process exits, so the last sample stands for the end.
            try:
                last_threads = thread_times(ps, ui_tid, dlls)
            except psutil.Error:
                pass
        if (window is not None and close_after is not None and now >= window + close_after) or now > timeout:
            proc.terminate()
            break
        try:
            peak = max(peak, ps.memory_info().peak_wset)
        except psutil.Error:
            pass
        time.sleep(0.005 if window is None else 0.25)
    proc.wait()
    end = time.perf_counter()
    cpu, dwm = cpu_seconds(proc._handle), dwm_seconds()
    by_name = {}
    if steady:
        span = end - steady[0]
        cpu, dwm = 100 * (cpu - steady[1]) / span, 100 * (dwm - steady[2]) / span
        for tid, (name, seconds) in last_threads.items():
            count, used = by_name.get(name, (0, 0.0))
            by_name[name] = (count + 1, used + 100 * (seconds - first_threads.get(tid, (name, 0.0))[1]) / span)
    else:
        cpu, dwm = 100 * cpu / (end - start), float("nan")
    return {
        "threads": sorted(((n, c, u) for n, (c, u) in by_name.items()), key=lambda t: -t[2]),
        "cpu": cpu,
        "dwm": dwm,
        "memory": peak / 2**20,
        "window": window if window is not None else float("nan"),
        "gpu": gpu.stop() if gpu else {},
    }


def vibelc_env(tmp):
    return dict(os.environ, VIBELC_CONFIG_DIR=os.path.join(tmp, "vibelc"))


def vibelc(clip, tmp, extra):
    r = play([VIBELC, "--new-window", "--fullscreen", "--quit-at-end", "--stats", *extra, clip], vibelc_env(tmp), tmp)
    text = open(os.path.join(tmp, "vibelc", "vibelc.log"), errors="replace").read()
    first = FIRST_FRAME.search(text)
    r["first"] = float(first.group(1)) if first else float("nan")
    counts = VIBELC_DROPS.findall(text)
    r["dropped"] = sum(map(int, counts[-1])) if counts else None
    hwdec = VIBELC_HWDEC.findall(text)
    r["decoder"] = hwdec[-1] if hwdec else "?"
    return r


def vlc(clip, tmp, sw):
    log = os.path.join(tmp, "vlc.log")
    cmd = VLC_BASE + (["--avcodec-hw=none"] if sw else []) + ["--fullscreen", "--play-and-exit", "--file-logging", f"--logfile={log}", "--log-verbose=2", clip]
    r = play(cmd, dict(os.environ), tmp)
    text = open(log, errors="replace").read() if os.path.exists(log) else ""
    r["dropped"] = len(VLC_DROPS.findall(text))
    r["first"] = float("nan")
    r["decoder"] = "d3d11va" if "D3D11VA" in text else ("dxva2" if "DXVA2" in text else "?")
    return r


def main():
    args = sys.argv[1:]
    runs, startup, players, extra, sw = 3, 0, [("VibeLC", vibelc), ("VLC", vlc)], [], False
    while args and args[0].startswith("--"):
        option = args.pop(0)
        if option == "--runs":
            runs = int(args.pop(0))
        elif option == "--startup":
            startup = int(args.pop(0))
        elif option == "--sw":
            sw = True
        elif option == "--threads":
            global THREADS
            THREADS = True
        elif option == "--only-vibelc":
            players = players[:1]
        elif option == "--vibelc-args":
            extra = shlex.split(args.pop(0))
        else:
            sys.exit(__doc__)
    if not args and not startup:
        sys.exit(__doc__)
    if sw:
        extra.append("--hwdec=no")
    if startup:
        print("start-up\tplayer\trun\twindow s\tpeak memory MB")
        for i in range(startup):
            for name, _ in players:
                with tempfile.TemporaryDirectory() as tmp:
                    if name == "VibeLC":
                        r = play([VIBELC, "--new-window", *extra], vibelc_env(tmp), tmp, close_after=1.0)
                    else:
                        r = play(VLC_BASE, dict(os.environ), tmp, close_after=1.0)
                print(f"start-up\t{name}\t{i + 1}\t{r['window']:.3f}\t{r['memory']:.0f}", flush=True)
                time.sleep(1)
    print("clip\tplayer\tdecoder\tCPU %\tdwm CPU %\tGPU % by engine\tdropped\tpeak memory MB\twindow s\tfirst frame s")
    for clip in args:
        for _ in range(runs):
            for name, player in players:
                with tempfile.TemporaryDirectory() as tmp:
                    r = player(os.path.abspath(clip), tmp, extra if player is vibelc else sw)
                gpu = " ".join(f"{k}={v:.0f}" for k, v in sorted(r["gpu"].items()) if v >= 0.5) or "-"
                print(f"{os.path.basename(clip)}\t{name}\t{r['decoder']}\t{r['cpu']:.1f}\t{r['dwm']:.1f}\t{gpu}"
                      f"\t{r['dropped']}\t{r['memory']:.0f}\t{r['window']:.3f}\t{r['first']:.3f}", flush=True)
                for thread, count, used in r["threads"]:
                    if used >= 0.1:
                        print(f"  thread\t{thread}\t{count}\t{used:.1f}", flush=True)
                if r["threads"]:
                    rest = r["cpu"] - sum(used for *_, used in r["threads"])
                    print(f"  thread\t(ended during the run, or exit)\t-\t{rest:.1f}", flush=True)
                time.sleep(2)


if __name__ == "__main__":
    main()
