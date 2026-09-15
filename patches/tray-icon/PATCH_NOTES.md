# Temporary tray-icon 0.24.2 Patch for macOS 27

## Context
On macOS 27, when an `NSMenu` is attached to `NSStatusItem` via `setMenu:`,
macOS 27's status item implementation intercepts left clicks to display the
menu natively, swallowing left-click events that should be forwarded to the
application's tray click handler even when `show_menu_on_left_click(false)` is set.

## Upstream Reference
- Pull Request: [tauri-apps/tray-icon#365](https://github.com/tauri-apps/tray-icon/pull/365)
  `fix(macos): restore tray click event forwarding on macOS 27`
- The upstream fix detaches the native `NSMenu` by default, attaching it transiently
  only while the menu is being presented (`show_menu` or right-click click handler),
  and detaching it immediately afterwards.

## Windows Dependency Pin
In `patches/tray-icon/Cargo.toml`, the Windows target dependency is pinned to
`windows-sys = "0.61"` (instead of the upstream range `>=0.60, <=0.61`) so
Cargo resolves `windows-sys 0.61.2`, avoiding unwanted dependency drift in
`Cargo.lock` against the rest of the workspace.

## How to Remove
Once Tauri / tray-icon releases a new version containing this fix:
1. Update `tray-icon` (or `tauri`) in `Cargo.lock` / `Cargo.toml`.
2. Remove the `[patch.crates-io]` entry for `tray-icon` in the workspace root `Cargo.toml`.
3. Delete the `patches/tray-icon` directory.
