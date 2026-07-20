#!/usr/bin/env bash

set -euo pipefail

if [[ "$(uname)" != "Darwin" ]]; then
    echo "error: macOS is required to build a Panes.app bundle" >&2
    exit 1
fi

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-"$root_dir/target"}"
dist_dir="${PANES_DIST_DIR:-"$root_dir/dist"}"
app_dir="$dist_dir/Panes.app"
contents_dir="$app_dir/Contents"
resources_dir="$contents_dir/Resources"
version="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root_dir/Cargo.toml" | head -n 1)"

if [[ -z "$version" ]]; then
    echo "error: could not determine the workspace version" >&2
    exit 1
fi

cargo build --manifest-path "$root_dir/Cargo.toml" --package panes-app --release --target-dir "$target_dir"

binary="$target_dir/release/panes"
if [[ ! -x "$binary" ]]; then
    echo "error: expected release binary at $binary" >&2
    exit 1
fi

rm -rf "$app_dir"
mkdir -p "$contents_dir/MacOS" "$resources_dir"
cp "$binary" "$contents_dir/MacOS/panes"
sed "s/@VERSION@/$version/g" "$root_dir/assets/macos/Info.plist" > "$contents_dir/Info.plist"

iconset_parent_dir="$(mktemp -d "${TMPDIR:-/tmp}/panes-iconset.XXXXXX")"
iconset_dir="$iconset_parent_dir/Panes.iconset"
mkdir "$iconset_dir"
cleanup() {
    rm -rf "$iconset_parent_dir"
}
trap cleanup EXIT

render_icon() {
    local filename="$1"
    local size="$2"
    sips -z "$size" "$size" "$rendered_icon" --out "$iconset_dir/$filename" >/dev/null
}

qlmanage -t -s 1024 -o "$iconset_dir" "$root_dir/assets/macos/AppIcon.svg" >/dev/null
rendered_icon="$iconset_dir/AppIcon.svg.png"
if [[ ! -f "$rendered_icon" ]]; then
    echo "error: Quick Look could not render the app icon" >&2
    exit 1
fi

render_icon icon_16x16.png 16
render_icon icon_16x16@2x.png 32
render_icon icon_32x32.png 32
render_icon icon_32x32@2x.png 64
render_icon icon_128x128.png 128
render_icon icon_128x128@2x.png 256
render_icon icon_256x256.png 256
render_icon icon_256x256@2x.png 512
render_icon icon_512x512.png 512
render_icon icon_512x512@2x.png 1024
rm "$rendered_icon"
iconutil --convert icns --output "$resources_dir/Panes.icns" "$iconset_dir"

plutil -lint "$contents_dir/Info.plist" >/dev/null
bundle_identifier="$(plutil -extract CFBundleIdentifier raw "$contents_dir/Info.plist")"
designated_requirement="=designated => identifier \"$bundle_identifier\""
codesign --force --deep --sign - \
    --identifier "$bundle_identifier" \
    --requirements "$designated_requirement" \
    "$app_dir"
codesign --verify --deep --strict "$app_dir"
echo "Created $app_dir"
