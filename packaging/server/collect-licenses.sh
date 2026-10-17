#!/bin/sh
# Run inside the Linux builder. Collect resolved production dependencies, not dev tooling.
set -eu
manifest=$1
destination=$2
mkdir -p "$destination"
cargo tree --manifest-path "$manifest" -p h-live-server --locked --edges normal,build --prefix none --format '{p}' > /tmp/license-tree
sed -nE 's/^([^ ]+) v([^ ]+).*$/\1-\2/p' /tmp/license-tree | sort -u > /tmp/license-packages
while IFS= read -r package; do
    case "$package" in h-live-*|qingkan-core-*) continue;; esac
    source=$(find "${CARGO_HOME:-/usr/local/cargo}/registry/src" -mindepth 2 -maxdepth 2 -type d -name "$package" -print -quit)
    [ -n "$source" ] || { echo "Missing source: $package" >&2; exit 1; }
    mkdir -p "$destination/$package"
    find "$source" -maxdepth 1 -type f \( -iname 'LICENSE*' -o -iname 'LICENCE*' -o -iname 'COPYING*' -o -iname 'NOTICE*' -o -iname 'COPYRIGHT*' -o -iname 'AUTHORS*' \) -exec cp '{}' "$destination/$package/" \;
    if [ -z "$(ls -A "$destination/$package")" ]; then
        if [ -d "/extra-licenses/$package" ]; then cp -r "/extra-licenses/$package/." "$destination/$package/";
        else find "$source" -type f \( -iname 'LICENSE*' -o -iname 'COPYING*' \) -exec cp '{}' "$destination/$package/" \;; fi
    fi
    [ -n "$(ls -A "$destination/$package")" ] || { echo "License review required: $package" >&2; exit 1; }
done < /tmp/license-packages
cp /tmp/license-tree "$destination/DEPENDENCIES.txt"
