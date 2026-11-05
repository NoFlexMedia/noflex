#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Regenerates assets/decoder-probes/: a few black frames per codec, resolution
# and bit depth. The app plays these hidden to measure what this device decodes
# in hardware. Run from flutter/ inside `nix develop` (ffmpeg-full).
set -euo pipefail
out=assets/decoder-probes
mkdir -p "$out"

make() { # name size pix_fmt codec-args...
  local name="$1" size="$2" fmt="$3"
  shift 3
  ffmpeg -hide_banner -loglevel error -y -f lavfi -i "color=black:size=$size:rate=24:duration=0.25" \
    -pix_fmt "$fmt" "$@" -an "$out/$name.mkv"
}

make h264-1080-8 1920x1080 yuv420p -c:v libx264 -preset medium -profile:v high -crf 40
make h264-2160-8 3840x2160 yuv420p -c:v libx264 -preset medium -profile:v high -crf 40
for depth in 8 10; do
  fmt=yuv420p
  [ "$depth" = 10 ] && fmt=yuv420p10le
  for size in 1920x1080:1080 3840x2160:2160; do
    res="${size%%:*}" h="${size##*:}"
    make "h265-$h-$depth" "$res" "$fmt" -c:v libx265 -preset fast -x265-params log-level=error:crf=40 -tag:v hvc1
    make "av1-$h-$depth" "$res" "$fmt" -c:v libsvtav1 -preset 8 -crf 55
    vp9_profile=0
    [ "$depth" = 10 ] && vp9_profile=2
    make "vp9-$h-$depth" "$res" "$fmt" -c:v libvpx-vp9 -profile:v "$vp9_profile" -row-mt 1 -speed 6 -b:v 0 -crf 55
  done
done
du -ch "$out"/* | tail -1
