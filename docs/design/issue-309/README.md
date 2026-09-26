# Handwritten Zenith Z

The user requested a less rigid, more handwritten logo after reviewing the split
Z in the Dock next to Muse. The reference informs the relaxed pen movement;
Zenith's Z is original vector geometry, with no copied Muse artwork or font.

The master remains `src-tauri/icons/zenith-mark.svg`: one filled outline with
curved entry/exit strokes, rounded terminals, and intentional width variation.
The shared generator preserves the same geometry across app bundles, compact UI,
favicon, README, and template tray. The cobalt palette and pale tile stay aligned
with the existing design contract. Native glass is unchanged.

`logo-preview.html` shows the app tile, 16/20/24/32/48 px compact marks, and 22 pt
monochrome mark against light and dark surfaces. Static artwork has no interaction
states. Historical issue-306 screenshots document the superseded design.

## Version and workflow

`just bump-patch` changed 0.3.61 to 0.3.62 once for this PR, and
`just check-version` confirmed the package, Tauri configuration, Cargo workspace,
and all three workspace lockfile entries agree. `AGENTS.md` now explicitly owns
version timing, issue/PR workflow, visual-contract preservation, and handoff
requirements. A PR template makes the version decision visible during review.

## Visual review

Reviewed on macOS 27.0 (26A428), 2026-09-26, application version 0.3.62:

- `logo-preview.png`: actual generated artwork at application and compact sizes,
  plus light/dark monochrome tray treatment.
- Review sheet checked at 320, 375, 414, and 768 px: all images loaded and no
  horizontal page overflow.
- `dashboard.png` and `quick-panel.png`: browser previews with mock IPC, showing
  the shared identity in app chrome. These do not validate native glass.

The installed app is not replaced by this PR. Native Dock cache refresh and
Windows rendering remain manual checks. Regenerating existing mobile assets does
not claim that Zenith supports mobile platforms.
