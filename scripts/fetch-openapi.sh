#!/usr/bin/env bash
# Downloads SoundCloud's official OpenAPI description into .cache/ (git-ignored).
#
# The soundcloud/api repository has no license, so we do not copy the file into this
# repository. We fetch it at a pinned commit and check its checksum instead, so that
# everyone (and CI) tests the mock against exactly the same description.
#
# To move to a newer version, change SHA and SUM together (review the diff first).
set -euo pipefail
cd "$(dirname "$0")/.."

SHA=5d407f4da0090df35e0482e176a60db2478443ef # soundcloud/api master, 2026-09-16
SUM=3b161a7f49f4c177a8b847e5a445bd0fb3ec68d6f4e6fb962d1fff41e935de48
dest=.cache/soundcloud-openapi/api.yaml
url="https://raw.githubusercontent.com/soundcloud/api/$SHA/openapi/api.yaml"

if [[ -f "$dest" ]] && echo "$SUM  $dest" | sha256sum --check --status; then
  exit 0 # already there and intact
fi

mkdir -p "$(dirname "$dest")"
curl --fail --silent --show-error --location --retry 3 --output "$dest.tmp" "$url"
if ! echo "$SUM  $dest.tmp" | sha256sum --check --status; then
  rm -f "$dest.tmp"
  echo "fetch-openapi: checksum mismatch for $url" >&2
  exit 1
fi
mv "$dest.tmp" "$dest"
echo "fetch-openapi: $dest ($SHA)"
