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

## Development

```bash
cargo test
cargo run -p panes-app
```
