#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-only
#
# Real encodes, since listing an encoder doesn't prove it runs. VideoToolbox is
# reported rather than required, because it needs a GPU that a CI VM may not expose.

set -euo pipefail

ffmpeg=$1
encode() {
    "$ffmpeg" -hide_banner -loglevel error -f lavfi -i testsrc2=s=1280x720:r=30 -frames:v 30 \
        -c:v "$@" -f null -
}
"$ffmpeg" -hide_banner -version | head -1
for encoder in libx264 libx265 libsvtav1; do
    encode "$encoder"
    echo "ok: $encoder"
done
# cpu_tonemap_filter in noflex-transcoder, on input tagged the way real HDR is.
"$ffmpeg" -hide_banner -loglevel error -f lavfi \
    -i 'testsrc2=s=640x360:r=30,format=yuv420p10le,setparams=color_primaries=bt2020:color_trc=smpte2084:colorspace=bt2020nc' \
    -frames:v 5 \
    -vf 'zscale=w=640:h=360:t=linear:npl=100:p=bt709,format=gbrpf32le,tonemap=tonemap=hable:desat=0,zscale=t=bt709:m=bt709:r=tv' \
    -f null -
echo "ok: CPU tone-mapping"
for encoder in h264_videotoolbox hevc_videotoolbox; do
    if encode "$encoder"; then echo "ok: $encoder"; else echo "UNAVAILABLE: $encoder"; fi
done
