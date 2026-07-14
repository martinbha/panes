# macOS Manual Validation Checklist

This checklist validates panes command behavior end to end on real macOS hardware.
Automated coverage for the same flows lives in
`crates/panes-runtime/tests/command_flow.rs` (executor + fake platform),
`crates/panes-core/src/layout.rs` (geometry), and
`crates/panes-macos/src/coordinates.rs` (native coordinate conversion), so this
document focuses on what only a real machine can verify: Accessibility behavior,
real application windows, and physical displays.

## Setup

1. Build the app: `cargo build -p panes-app`.
2. Quit any running Panes process, then run `tccutil reset Accessibility`.
3. Start the app with `cargo run -p panes-app` and confirm the system asks for
   Accessibility permission once.
4. Confirm the tray shows **Grant Accessibility Permission…** while access is
   missing. Use it to open System Settings → Privacy & Security →
   Accessibility, then grant access.
5. Confirm the tray changes to **Accessibility Permission Granted** without a
   restart and that subsequent commands do not repeat the system prompt.
6. Rebuilt dev binaries can silently lose trust; re-grant if commands stop
   working.
7. Confirm the panes tray icon also includes command submenus and Quit.
8. Use a normal resizable app window (e.g. TextEdit, Safari) as the target.

For every command, verify both invocation paths at least once per session:
the default hotkey and the tray menu item must behave identically.

## Command Matrix — Single Display

Expected behavior uses the default config (no gaps, 0.5 splits).

| Command | Default hotkey | Expected result | Pass |
| --- | --- | --- | --- |
| Left Half | ⌃⌥← | Window fills left half of the work area | ☐ |
| Right Half | ⌃⌥→ | Window fills right half | ☐ |
| Center Half | menu only | Half-width window centered, full height | ☐ |
| Top Half | ⌃⌥↑ | Window fills top half | ☐ |
| Bottom Half | ⌃⌥↓ | Window fills bottom half | ☐ |
| Top Left | ⌃⌥U | Window fills top-left quarter | ☐ |
| Top Right | ⌃⌥I | Window fills top-right quarter | ☐ |
| Bottom Left | ⌃⌥J | Window fills bottom-left quarter | ☐ |
| Bottom Right | ⌃⌥K | Window fills bottom-right quarter | ☐ |
| First Third | ⌃⌥1 | Left third (top third on portrait displays) | ☐ |
| Center Third | ⌃⌥2 | Middle third | ☐ |
| Last Third | ⌃⌥3 | Right third (bottom third on portrait) | ☐ |
| First Two Thirds | ⌃⌥4 | Left two thirds | ☐ |
| Center Two Thirds | ⌃⌥5 | Centered two thirds | ☐ |
| Last Two Thirds | ⌃⌥6 | Right two thirds | ☐ |
| Maximize | ⌃⌥⏎ | Window fills the work area (not macOS full screen) | ☐ |
| Almost Maximize | ⌃⌥A | 90% of work area, centered | ☐ |
| Maximize Height | ⌃⌥H | Full height, width and x unchanged | ☐ |
| Center | ⌃⌥C | Window centered, size unchanged | ☐ |
| Restore | ⌃⌥⌫ | Window returns to its rect before the first panes command | ☐ |
| Move Left | ⌃⌥⇧← | Window flush against left edge, size unchanged | ☐ |
| Move Right | ⌃⌥⇧→ | Window flush against right edge | ☐ |
| Move Up | ⌃⌥⇧↑ | Window flush against top edge | ☐ |
| Move Down | ⌃⌥⇧↓ | Window flush against bottom edge | ☐ |
| Grow | ⌃⌥= | Window grows by the resize step from its center | ☐ |
| Shrink | ⌃⌥- | Window shrinks by the resize step toward its center | ☐ |

## Restore Semantics

| Scenario | Expected result | Pass |
| --- | --- | --- |
| One command, then Restore | Original rect restored, second Restore reports no restore rect | ☐ |
| Several commands, then Restore | Rect from before the *first* command restored | ☐ |
| Restore with no prior command | Nothing moves; error logged to console | ☐ |

## Multi-Display

Requires at least two displays. Repeat with displays arranged side by side.

| Scenario | Expected result | Pass |
| --- | --- | --- |
| Tiling command on secondary display | Rect computed against that display's work area | ☐ |
| Window spanning both displays | Command targets the display with the larger overlap | ☐ |
| Focused window on display A, cursor on display B | Commands act on the focused window's display | ☐ |
| Displays with different resolutions | No offset drift; window lands inside the target work area | ☐ |
| Menu bar / Dock on either display | Work area excludes them on every display | ☐ |
| Repeated Left/Right Half | `left → right → next display left → next display right`; wraps at the outer edge | ☐ |
| Repeated Move at a display edge | Window enters the adjacent display at its near edge and wraps at the outer edge | ☐ |

## Application Compatibility

| Scenario | Expected result | Pass |
| --- | --- | --- |
| Chrome or a Chrome PWA | Window can be moved and resized | ☐ |
| Fixed-size or minimum-size app (e.g. Activity Monitor) | Window moves into the requested zone and remains on-screen at an allowed size | ☐ |
| System Settings | Window moves; any macOS-enforced size limit is preserved | ☐ |

## Gaps and Ratios

Gaps and split ratios currently require code changes (persistent config is #5).
Validate by running with a modified `LayoutConfig` in a dev build.

| Scenario | Expected result | Pass |
| --- | --- | --- |
| Gap enabled, Left Half + Right Half | Both windows inset; see limitation on seam width below | ☐ |
| Gap enabled, Center / Move / Grow | No gap applied to non-tiling commands | ☐ |
| Custom horizontal split (e.g. 0.6) | Left/right halves and corners use the ratio | ☐ |

## Unsupported Windows

| Scenario | Expected result | Pass |
| --- | --- | --- |
| No window focused (Finder desktop) | Error logged, no crash | ☐ |
| Minimized window | Command refused, no crash | ☐ |
| Non-resizable window | Window moves within its allowed size, no crash | ☐ |
| macOS full-screen window | Command refused or no-op, no crash | ☐ |

## Known Limitations

- Adjacent tiled windows get a doubled gap seam
  ([#12](https://github.com/martinbha/panes/issues/12)).
- Size cycling is not implemented for repeated commands
  ([#13](https://github.com/martinbha/panes/issues/13)).
- Grow can push a window past the screen edge, and display attach/detach and
  offscreen safety are not hardened yet
  ([#8](https://github.com/martinbha/panes/issues/8)).
- No scriptable driver exists yet to automate this checklist
  ([#18](https://github.com/martinbha/panes/issues/18)).
