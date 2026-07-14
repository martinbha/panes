#!/usr/bin/env bash

set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
    echo "error: macOS is required to package Panes.app" >&2
    exit 1
fi

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dist_dir="${PANES_DIST_DIR:-"$root_dir/dist"}"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root_dir/Cargo.toml" | head -n 1)"

if [[ -z "$version" ]]; then
    echo "error: could not determine the workspace version" >&2
    exit 1
fi

case "$(uname -m)" in
    arm64) architecture="arm64" ;;
    x86_64) architecture="x86_64" ;;
    *)
        echo "error: unsupported macOS architecture: $(uname -m)" >&2
        exit 1
        ;;
esac

PANES_DIST_DIR="$dist_dir" "$root_dir/scripts/bundle-macos.sh"

archive="$dist_dir/panes-v$version-macos-$architecture.zip"
rm -f "$archive"
ditto -c -k --sequesterRsrc --keepParent "$dist_dir/Panes.app" "$archive"

echo "Created $archive"
