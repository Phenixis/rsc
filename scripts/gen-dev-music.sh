#!/usr/bin/env bash
# Generates a short, recognisable test playlist for the local backend: one sine tone
# per track, each at a different pitch, so you can hear the playback order.
#
#   scripts/gen-dev-music.sh [library-dir] [tracks] [seconds]
#
# Defaults: ~/Music/rsc-dev, 5 tracks of 8 seconds -> <library>/test-playlist/
set -euo pipefail

library="${1:-$HOME/Music/rsc-dev}"
tracks="${2:-5}"
seconds="${3:-8}"
dir="$library/test-playlist"

mkdir -p "$dir"
for i in $(seq 1 "$tracks"); do
  ffmpeg -loglevel error -y -f lavfi -i "sine=frequency=$((330 + i * 110)):duration=$seconds" \
    -c:a libmp3lame -q:a 4 "$dir/$(printf '%02d' "$i")-tone-$i.mp3"
done
echo "Wrote $tracks tracks to $dir"
