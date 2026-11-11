#!/bin/sh
# Rebuilds lucide-subset.ttf: the Lucide icon font (ISC, see LICENSE-lucide.txt) reduced to the
# glyphs VibeLC uses. Needs curl and fontTools (pip install fonttools). The codepoints must match
# src/ui/icons.rs, which a unit test checks against the font.
set -eu
VERSION=1.71.0
GLYPHS="play pause square skip-back skip-forward maximize minimize sliders-horizontal list-video
repeat repeat-1 shuffle volume-2 volume-1 volume volume-x camera folder-open link info settings x
plus trash-2 check step-forward step-back captions audio-lines film search clipboard history keyboard
pin list-plus folder-plus clock save gauge monitor-play circle-alert chevron-right"
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -sL "https://registry.npmjs.org/lucide-static/-/lucide-static-$VERSION.tgz" | tar -xz -C "$tmp"
unicodes=$(python3 -I -c '
import json, sys
cp = json.load(open(sys.argv[1]))
print(",".join("U+%04X" % cp[name] for name in sys.argv[2].split()))
' "$tmp/package/font/codepoints.json" "$GLYPHS")
python3 -I -m fontTools.subset "$tmp/package/font/lucide.ttf" --unicodes="$unicodes" \
    --output-file=lucide-subset.ttf --no-hinting --desubroutinize
cp "$tmp/package/LICENSE" LICENSE-lucide.txt
ls -l lucide-subset.ttf
