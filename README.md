# panes

`panes` is a cross-platform window management app written in Rust.

The first milestone is intentionally narrow:

- native tray/menu shell,
- keyboard/menu-triggered window commands,
- shared layout logic for macOS and Windows,
- platform adapters that find the active window, detect displays, and apply calculated window rectangles.

Drag-to-snap, richer preferences, and platform-specific polish come after the keyboard/menu path is working.

## Workspace

- `crates/panes-core`: platform-neutral commands, geometry, layout calculations, and window history.
- `crates/panes-platform`: traits for native platform adapters.
- `crates/panes-macos`: macOS adapter placeholder.
- `crates/panes-windows`: Windows adapter placeholder.
- `crates/panes-app`: app entry point.

## Configuration

panes reads an optional TOML config file at startup:

- macOS: `~/Library/Application Support/panes/config.toml`
- Windows: `%APPDATA%\panes\config.toml`
- Linux: `~/.config/panes/config.toml`

A missing file means built-in defaults. See
[docs/config.example.toml](docs/config.example.toml) for all supported keys:
layout settings (gap, split ratios, almost-maximize size, resize step),
per-command hotkey overrides, and disabled commands.

Invalid individual values fall back to their defaults with a warning on
stderr; an unparseable file falls back to full defaults with an error naming
the file and problem.

## Development

```bash
cargo test
cargo run -p panes-app
```

## macOS app bundle

On macOS, create a double-clickable release app with:

```bash
scripts/bundle-macos.sh
```

The command creates `dist/Panes.app`. It is an unsigned local build intended
for development and manual testing; drag it to `/Applications` if you want to
keep it there. Panes runs as a menu-bar-only app, so it does not appear in the
Dock.

The first launch requires Accessibility access for `Panes` in System Settings
→ Privacy & Security → Accessibility. Rebuilds and changes to the app’s
location can require granting access again.

For an app intended to leave the development machine, sign and notarize the
bundle with an Apple Developer certificate before distributing it.
