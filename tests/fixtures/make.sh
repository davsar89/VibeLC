#!/bin/sh
# Regenerates the small media fixtures used by the integration tests (needs ffmpeg with libx264,
# libopus and libvorbis). The outputs are committed, so running this is only needed to change them.
set -eu
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/subs.srt" <<'EOF'
1
00:00:00,200 --> 00:00:02,800
VibeLC subtitle test
EOF

cat > "$tmp/meta.txt" <<'EOF'
;FFMETADATA1
title=VibeLC fixture
[CHAPTER]
TIMEBASE=1/1000
START=0
END=1500
title=Opening
[CHAPTER]
TIMEBASE=1/1000
START=1500
END=3000
title=Ending
EOF

# 3 s, 320x180 H.264, two audio tracks (eng 440 Hz, nor 660 Hz), one SRT track, two chapters.
ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i testsrc2=size=320x180:rate=25:duration=3 \
    -f lavfi -i sine=frequency=440:duration=3 \
    -f lavfi -i sine=frequency=660:duration=3 \
    -i "$tmp/subs.srt" -i "$tmp/meta.txt" \
    -map 0:v -map 1:a -map 2:a -map 3:s -map_metadata 4 -map_chapters 4 \
    -c:v libx264 -preset veryfast -crf 32 -pix_fmt yuv420p \
    -c:a libopus -b:a 24k -c:s srt \
    -metadata:s:a:0 language=eng -metadata:s:a:0 title="English tone" \
    -metadata:s:a:1 language=nor -metadata:s:a:1 title="Norwegian tone" \
    -metadata:s:s:0 language=eng \
    av.mkv

# 1.5 s, 160x90 H.264 + AAC: a second playlist entry.
ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i testsrc=size=160x90:rate=25:duration=1.5 \
    -f lavfi -i sine=frequency=330:duration=1.5 \
    -c:v libx264 -preset veryfast -crf 32 -pix_fmt yuv420p -c:a aac -b:a 32k \
    -metadata title="Second fixture" \
    second.mp4

# 2 s audio-only file with tags.
ffmpeg -hide_banner -loglevel error -y \
    -f lavfi -i sine=frequency=523:duration=2 \
    -c:a libvorbis -q:a 0 -metadata title="VibeLC tone" -metadata artist="VibeLC tests" \
    tone.ogg

# An external subtitle file for "Add Subtitle File".
printf '1\n00:00:00,500 --> 00:00:02,500\nVibeLC external subtitle\n' > extra.srt

ls -l av.mkv second.mp4 tone.ogg extra.srt
