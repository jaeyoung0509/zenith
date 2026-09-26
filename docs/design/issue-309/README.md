# Blue ribbon Zenith Z

The user rejected the first handwritten draft and approved the second of three
supplied ribbon references. This revision follows that reference: two straight
horizontal bars with rounded ends, one broad diagonal, restrained blue gradients,
and a cool near-white tile. The earlier handwritten direction is superseded.

`src-tauri/icons/zenith-mark.svg` owns the three filled ribbon paths and their
gradients. `scripts/generate_icons.mjs` places that artwork on the app/compact
tiles and derives a black-alpha template from the exact same three paths. No
separate menu-bar drawing is maintained. Window materials and app UI tokens are
unchanged.

`logo-preview.html` shows the app tile, 16/20/24/32/48 px compact marks, and 22 pt
monochrome mark against light and dark surfaces. Static artwork has no interaction
states. Historical issue-306 screenshots document the superseded split design.

## Version and workflow

This is a follow-up on the same PR. The existing `just bump-patch` transition
**0.3.61 → 0.3.62** remains; no second increment is made. `just check-version`
checks the package, Tauri configuration, Cargo workspace, and all three workspace
lockfile entries. AGENTS.md and the PR template retain the version and handoff
rules added in this PR.

## Visual review

Reviewed on macOS 27.0 (26A428), 2026-09-26, application version 0.3.62:

- `logo-preview.png`: generated app/compact artwork and light/dark monochrome tray.
- Review sheet checked at 320, 375, 414, and 768 px for loaded images and overflow.
- `dashboard.png` and `quick-panel.png`: browser previews with mock IPC, showing
  the shared identity in app chrome; these do not validate native glass.

The installed app is not replaced by this PR. Dock cache refresh and Windows
native rendering remain manual checks. Regenerating existing mobile assets does
not claim mobile application support. The PR records final check/build results.
