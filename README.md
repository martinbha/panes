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
- `crates/panes-macos`: macOS accessibility, display, tray, and hotkey adapter.
- `crates/panes-windows`: Windows display, window, tray, and hotkey adapter.
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

## Window-management failures

Transient desktop states—no focused window, no saved restore rectangle, or an
unsupported/vanished window—are ignored in release builds and emitted as
structured `event=command_failure` diagnostics in development builds.
Permission, native API, and display-geometry failures are also logged in
release builds. Command failures do not show tray notifications because common
focus and window-state changes would make them noisy during normal use.

Restore history is intentionally conservative. It is discarded when a native
window identifier is observed with different application/window metadata or
when the connected display geometry changes. Restore rectangles are fitted to
the current work area so stale coordinates cannot place a window offscreen.

## Development

The workspace requires Rust 1.85.0. `rust-toolchain.toml` pins that release and
installs `rustfmt` and Clippy through rustup. Keep its channel aligned with the
workspace `rust-version` when raising the minimum supported compiler.

Clone the repository, then run the complete local check set:

```bash
cargo fmt --all -- --check
cargo check --workspace --all-targets
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-targets
```

On macOS, install the Xcode command-line tools, then start the menu-bar app with
`cargo run -p panes-app`. Building the app bundle additionally requires the
standard `qlmanage`, `sips`, `iconutil`, and `plutil` tools included with macOS.

On 64-bit Windows 10 or 11, install rustup with the MSVC toolchain and Visual
Studio Build Tools with the Windows SDK, then run `cargo run -p panes-app` from
PowerShell. The Windows SDK resource compiler embeds the application manifest
and version metadata in release builds.

Pull requests and pushes to `main` run the same formatting, check, lint, and
test gates on macOS arm64 and Windows x86_64. Successful jobs also upload
versioned platform packages. See [docs/releasing.md](docs/releasing.md) for the
artifact contract and deferred distribution work.

## macOS Accessibility permission

Panes needs macOS Accessibility permission to inspect and move application
windows. On the first launch, it requests access with the system prompt. While
access is missing, the tray shows **Grant Accessibility Permission…**; choose
it to open System Settings → Privacy & Security → Accessibility.

Panes checks for the grant in the background and becomes ready without a
restart. The tray item changes to **Accessibility Permission Granted** when the
process is trusted. Window commands also perform a non-prompting trust check,
so using a command never repeats the system prompt.

For permission-flow testing, quit Panes and reset Accessibility consent:

```bash
# Reset all Accessibility grants (useful for `cargo run` development builds).
tccutil reset Accessibility

# Reset only the bundled app.
tccutil reset Accessibility io.github.martinbha.panes
```

Accessibility trust is tied to the app's identity, signature, location, and
binary. Rebuilding a development binary or moving the app can silently remove
trust; reset or re-grant access if window commands stop working after a build.

## macOS app bundle

On macOS, create a double-clickable release app with:

```bash
scripts/bundle-macos.sh
```

The command creates `dist/Panes.app`. It is an unsigned local build intended
for development and manual testing; drag it to `/Applications` if you want to
keep it there. Panes runs as a menu-bar-only app, so it does not appear in the
Dock.

Create the versioned ZIP used by CI with:

```bash
scripts/package-macos.sh
```

See [macOS Accessibility permission](#macos-accessibility-permission) for the
first-launch flow and development reset commands.

For an app intended to leave the development machine, sign and notarize the
bundle with an Apple Developer certificate before distributing it.

## Windows package

On 64-bit Windows, build a release executable and versioned portable ZIP from
PowerShell:

```powershell
scripts/package-windows.ps1
```

The package is written to `dist` and contains `panes.exe` plus the README. The
release executable is a tray application, carries version and DPI-awareness
metadata, and does not open a console window. It is currently unsigned and is
distributed as a portable ZIP; an installer is intentionally deferred until
the release and signing process is established.
