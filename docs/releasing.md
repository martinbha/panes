# Packaging and release artifacts

Panes currently produces portable development artifacts in continuous
integration. The workflow validates and packages the native application on the
following hosted runners:

| Platform | Runner | Architecture | Artifact filename |
| --- | --- | --- | --- |
| macOS | macOS 15 | arm64 | `panes-v<VERSION>-macos-arm64.zip` |
| Windows | Windows 2025 | x86_64 | `panes-v<VERSION>-windows-x86_64.zip` |

The version comes from `[workspace.package].version` in the root `Cargo.toml`.
Generated bundles, binaries, and archives remain under the ignored `dist` and
`target` directories locally. CI uploads only the final versioned ZIP files.

## Local packaging

On macOS:

```bash
scripts/package-macos.sh
```

This builds `Panes.app`, renders the checked-in SVG icon to `Panes.icns`,
validates `Info.plist`, applies a stable ad-hoc signature for local development,
and preserves the executable mode in the ZIP archive. The ad-hoc signature
helps retain Accessibility permission across same-location rebuilds; it does
not replace Developer ID signing or notarization for public distribution.

On 64-bit Windows PowerShell:

```powershell
scripts/package-windows.ps1
```

This builds `panes.exe` with embedded version information and a per-monitor-v2
DPI-aware manifest, then packages the executable and README as a portable ZIP.
The build requires the MSVC Rust toolchain and a Windows SDK installation so
the resource compiler is available.

Both scripts honor the repository's pinned Rust 1.85.0 toolchain. The macOS
script accepts `PANES_DIST_DIR` and `CARGO_TARGET_DIR`; the Windows script
accepts `-DistDir` and `-TargetDir` parameters for isolated builds.

## Distribution status

These artifacts are intended for development and manual validation. Before a
public release:

- sign and notarize the macOS app with an Apple Developer identity;
- sign the Windows executable with an Authenticode certificate;
- decide whether Windows needs an MSI or MSIX installer after the portable ZIP
  layout and upgrade behavior are stable;
- add a tag-triggered release workflow after native smoke testing is part of
  the release checklist;
- add macOS x86_64 or Windows arm64 artifacts when those architectures become
  supported release targets.

Do not commit generated artifacts. Release automation should attach the same
versioned ZIP files produced by the packaging scripts.
