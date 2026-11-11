# VibeLC and VLC on an old desktop

(Linux, 2014 desktop. The Windows measurements are at the end, under "Windows on a 2024 laptop".)

## Setup

- **Machine:** Intel Core i7-4790 (Haswell, 2014; 4 cores, 8 threads), Intel HD Graphics 4600,
  31 GiB RAM, Ubuntu 22.04 with X11 at 1920×1200, Mesa 23.2, VA-API driver i965 2.4.
- **Players:** VibeLC 0.1.0 from `packaging/linux/build-deb.sh` (its own libmpv: mpv 0.42.0,
  FFmpeg 8.1.3, dav1d 1.5.4) and VLC 3.0.16 from Ubuntu 22.04 (FFmpeg 4.4, dav1d 0.9.2).
- **Clips:** Big Buck Bunny (© Blender Foundation, CC BY 3.0), seconds 60–120 of the official
  1080p and 2160p 30 fps H.264 encodes with their MP3 track. The 1080p segment was also
  re-encoded to MPEG-2 (15 Mbit/s, MP2 audio, in MPEG-TS like a TV recording), HEVC (x265,
  CRF 24) and AV1 (libaom, CRF 34). The HD 4600 decodes H.264 and MPEG-2 in hardware, and both
  players use it for them; it has no HEVC or AV1 decoder, so both players decode those in
  software (FFmpeg and dav1d).
- **Run:** `bench/compare.py --runs 2` plays each clip to its end, fullscreen, with sound
  through PulseAudio, first in VibeLC and then in VLC, each with default settings in a fresh
  configuration (VLC's first-run privacy question is pre-answered).

## Measures

- **CPU:** user plus system CPU time of the player process divided by the wall time, in percent
  of one core.
- **Dropped:** frames not shown. For VibeLC, mpv's dropped and decoder-dropped frame counters;
  for VLC, its "picture is too late to be displayed" warnings in the verbose log.
- **Window:** seconds from launch until a window is visible.
- The script also reports the whole machine's CPU. It was 210–270% of a core for both players
  in every run, mostly the remote desktop service encoding the changing screen, so it is left
  out below.

## Results

CPU and dropped cells give both runs; memory and window cells give the range of the two.

| Clip | Decoded by | CPU % VibeLC | CPU % VLC | Dropped VibeLC | Dropped VLC | Peak memory MB VibeLC | VLC | Window s VibeLC | VLC |
|---|---|---|---|---|---|---|---|---|---|
| 1080p H.264 | graphics card | 10, 10 | 10, 10 | 1, 1 | 91, 91 | 173 | 246–247 | 0.57 | 0.67 |
| 2160p H.264 | graphics card | 10, 11 | 11, 11 | 1, 1 | 96, 92 | 240 | 355–356 | 0.57–0.62 | 0.62 |
| 1080p MPEG-2 | graphics card | 12, 12 | 11, 11 | 1, 1 | 0, 0 | 152–154 | 204 | 0.57 | 0.62–0.72 |
| 1080p HEVC | processor | 41, 42 | 45, 46 | 1, 1 | 0, 0 | 207 | 261–262 | 0.57 | 0.67–0.72 |
| 1080p AV1 | processor | 59, 60 | 51, 52 | 1, 1 | 0, 0 | 221 | 301 | 0.57 | 0.67 |

- **Video the graphics card decodes:** both players decode with VA-API and draw the frames
  zero-copy through EGL, and use the same CPU within a point. VLC shows fewer frames: in every
  H.264 run it gave up on 91–96 pictures as too late (5% of the 1,800, one every 0.6 s on
  average). VibeLC dropped one, at start-up.
- **HEVC:** VibeLC uses 9% less CPU. FFmpeg decodes straight into graphics memory (see
  "direct rendering" below), so each frame skips a copy.
- **AV1:** VibeLC uses 15% more CPU; the next section explains why.
- **Memory and start-up:** VibeLC's peak memory is 21–33% lower, and its window appears about
  0.1 s sooner (0–0.15 s).

## Why AV1 costs VibeLC more

VibeLC's dav1d is a newer version, run with mpv's default thread count (logical processors plus
one: 9 here), and on this 4-core, 8-thread processor those threads share cores. Decoding the
60 s clip flat out with dav1d's own command-line tool, then paced at 30 frames per second as
in playback (`--realtime`, first 30 s):

| dav1d | Threads | Flat out: CPU time | Flat out: wall time | Paced: CPU |
|---|---|---|---|---|
| 0.9.2 (VLC's) | 8 frame, 4 tile (VLC's choice) | 20.5–21.3 s | 3.6–3.8 s | 69–75% |
| 1.5.4 (VibeLC's) | 9 (mpv's default) | 21.7–23.6 s (8 threads) | 3.4–3.75 s (8 threads) | 91–92% |
| 1.5.4 | 4 | 13.8–15.8 s | 3.7–4.3 s | 57% |

Per frame, dav1d 1.5.4 with 4 threads is the cheapest of all, and the threads beyond the 4
cores buy about 10% speed for about 50% more CPU time. In VibeLC, 4 threads cut AV1 playback from
about 68% to 62% of a core. VibeLC still keeps mpv's default, because fewer threads also lower
the peak decoding speed that a weak machine needs: FFmpeg decodes the HEVC clip in 3.8–4.8 s
with 9 threads and in 5.1 s with 4, while in playback HEVC costs the same either way (46–48%).
A dual-core laptop with hyperthreading that plays 4K in software needs that headroom.

## What the X server costs both players

On this desktop the Intel X driver (SNA with TearFree, which offers DRI2 but not DRI3) gives a
fullscreen OpenGL window a new 9 MB screen buffer about every other frame: kernel tracing
counted 46 in 3 s during VibeLC's playback and 45 during VLC's. The player's own thread pays
for clearing each one: about 2.4% of a core for VibeLC (perf), half of its main thread's time
at 1080p. Enabling DRI3 in the X configuration may avoid this; it was not tested.

## Changes these measurements led to

Measured with 20 s of per-thread CPU sampling during playback, unless noted.

| Change | Effect |
|---|---|
| A new video frame reuses the last interface layout also in fullscreen: the hidden seek bar no longer asked for a new layout 30 times a second | 1080p H.264: 10.6 → 9.9%; 2160p: 12.6 → 11.1% |
| MPEG-2 and WMV 9 decoded by the graphics card, as VLC does (mpv's default list leaves them to the processor); the card's MPEG-2 pictures match FFmpeg's software decoding at 67 dB PSNR | 1080p MPEG-2, 10 s clip, whole run: 34 → 15% |
| Direct rendering: FFmpeg decodes into graphics memory. mpv allows it only on AMD and NVIDIA, assuming other drivers map that memory uncached; VibeLC times a read of such memory at start and enables it when it is cached (on this Intel card it is) | HEVC: main thread 9.7 → 5.1% of a core, total about 52 → 47% |
| Frames that can't use direct rendering (AV1 via dav1d, copies back from the graphics card) are uploaded through pixel buffers, as VLC does | AV1: main thread 10.6 → 7.6% of a core |
| PulseAudio buffer of 200 ms instead of mpv's 100 ms; volume and equalizer changes now lag by up to 0.2 s | 1080p H.264: 9.8–10.1 → 8.7–8.8% |
| Whole-program optimization (fat LTO, one code generation unit) | within noise: VibeLC's own code is about 2% of its CPU time |

## Caveats

- One machine. Other cards and drivers will differ, and Windows most: there VibeLC decodes with
  D3D11 and copies each frame back on non-NVIDIA cards, which was not measured.
- The processor's clock follows its load (schedutil), so CPU % is not pure work: dav1d needs
  about 25% of a core to decode the AV1 clip flat out with 4 threads but about 50% during
  playback, where it runs in short bursts at lower clocks.
- VLC's dropped frames are counted from its verbose log, which costs it a little CPU; VibeLC
  runs with its once-a-second statistics poll.
- Whole-run CPU includes start-up, about a second of the run.

# Windows on a 2024 laptop

## Setup

- **Machine:** Intel Core Ultra 7 258V (Lunar Lake; 4 performance and 4 efficient cores), Intel
  Arc 140V graphics (driver 32.0.101.8132), Windows 11, on mains power. The desktop was in use
  during the runs (a remote-desktop session in a browser), so the numbers carry about ±3 points
  of noise; the players ran alternately, so both saw the same background load.
- **Players:** VibeLC 1.0.1 built with `cargo build --release` (MSVC), with `libmpv-2.dll` from
  shinchiro's mpv builds (2026-11-11), drawing with Direct3D 11; VLC 3.0.23. The Arc 140V decodes
  every clip below in hardware, and both players use it for all of them, without copying the
  frames back to memory (`d3d11va`).
- **Clips:** the same Big Buck Bunny segments as above (1080p and 2160p H.264, 1080p MPEG-2 in
  MPEG-TS, 1080p HEVC with x265 CRF 24, 1080p AV1 with libaom CRF 34), plus 1080p VP9 (libvpx,
  CRF 33) and 2160p 10-bit HEVC (x265 CRF 24).
- **Run:** `python bench/compare_win.py --startup 5 --runs 3 CLIP...` plays each clip to its end,
  fullscreen with sound, in VibeLC and then in VLC, both with default settings (VibeLC in a fresh
  configuration folder, VLC with `--ignore-config`). CPU is the player process's CPU time from
  3 s after its window appears to the end, over that wall time, in percent of one core: playback
  only, without start-up.

## Results

| Clip | CPU % VibeLC | VLC | Peak memory MB VibeLC | VLC | Dropped VibeLC | VLC |
|---|---|---|---|---|---|---|
| 1080p H.264 | 10, 9, 11 | 7, 6, 6 | 341–342 | 467–474 | 1 | 93–94 |
| 2160p H.264 | 10, 11, 13 | 5, 7, 5 | 818–827 | 1135–1148 | 1 | 83–89 |
| 1080p MPEG-2 | 9, 10, 9 | 5, 9, 9 | 190–192 | 247–250 | 1 | 0 |
| 1080p HEVC | 9, 13, 9 | 7, 9, 9 | 215–219 | 384–387 | 1 | 0 |
| 2160p HEVC 10-bit | 9, 11, 10 | 9, 10, 8 | 354–376 | 1329–1364 | 1 | 0 |
| 1080p VP9 | 8, 7, 9 | 6, 5, 7 | 217–218 | 265–271 | 1 | 0 |
| 1080p AV1 | 9, 10, 9 | 8, 6, 7 | 201–203 | 245–248 | 1 | 0 |

- **Dropped frames:** VibeLC's one is at start-up. VLC's are its "picture is too late" drops,
  in every H.264 run, as on Linux; they also spare it the work of showing those frames.
- **Start-up:** with no file, VibeLC's window appears after 0.037–0.053 s and VLC's after
  0.093–0.129 s (5 runs each). VibeLC's first video frame follows 0.51–0.55 s after its window
  (0.8–1.0 s for 1080p H.264, 1.4–1.5 s for 2160p H.264). With no file VibeLC uses 125 MB against
  VLC's 41 MB: `libmpv-2.dll` (121 MB, every mpv feature and library linked in) is mapped at start.
- **GPU:** Windows' per-process "3D" engine use was 2–9% for VibeLC and 1–8% for VLC. The
  "VideoDecode" counter read 100% for every player and clip, which this driver reports for any
  use, so it is left out.
- **Software decoding** (`--sw`: `--hwdec=no` and `--avcodec-hw=none`, 2 runs each): VibeLC
  1.0.1, which turns on FFmpeg's `fast` shortcuts and skips deblocking non-reference frames, used
  15–16% of a core on 1080p HEVC (VLC 15–16%), 18–20% on AV1 (VLC 19%) and 29% on 2160p H.264
  (VLC 29–43%, with 94 drops). Without those two options VibeLC used 17–20%, 21–24% and 32–38%.

VibeLC plays everything here with VLC's dropped-frame-free smoothness or better, in 15–75% less
memory, and starts 2–3 times faster; VLC uses 1–5 points less of one core during hardware-decoded
playback.

### Earlier output paths

Before Direct3D 11, VibeLC drew with OpenGL. With the driver's OpenGL (WGL) this Intel card has
no copy-free path (mpv's `dxva2-dxinterop` needs `WGL_NV_DX_interop`, which the driver lacks), so
mpv used `d3d11va-copy`: 18–36% of a core where Direct3D 11 needs 9–13%. ANGLE's OpenGL ES over
Direct3D 11 avoided the copy (11–16%), but its DLLs are no longer distributed on their own, it
kept the GPU busier on software-decoded video, and it used about 100 MB more. Those runs measured
CPU over the whole run, start-up included, so they are not exactly comparable with the table.
`--gl=wgl` still selects the OpenGL path.

## Caveats

- One machine, a busy desktop, three runs per clip (two in software).
- In software decoding the Direct3D 11 renderer uploads frames with a copy, where OpenGL with
  cached mapped buffers lets FFmpeg decode straight into graphics memory; on these clips the two
  measured the same within the noise.
