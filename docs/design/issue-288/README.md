# Issue 288 interface audit

This is the review record for Zenith 0.3.57. The originating 0.3.56 report is
recorded in [issue 288](https://github.com/jaeyoung0509/zenith/issues/288).
The images below are **browser previews with mock data**. They demonstrate
layout and state copy, not macOS vibrancy or native resize behavior.

## Validation environment

| Run | Date (KST) | Version | OS and display | Size | Appearance and result |
| --- | --- | --- | --- | --- | --- |
| Browser preview | 2026-09-26 | 0.3.57 | Chromium on macOS 27.0 (26A428), built-in 2560 × 1600 Retina | Main 800 × 560; Quick 320, 360, 400 × 740 | Light, mock IPC. Navigation, visible layout, and working copy reviewed. |
| Native QA bundle | 2026-09-26 | 0.3.57 | macOS 27.0 (26A428), built-in 2560 × 1600 Retina, 2× scale | Main 960 × 660; Quick 400 × 740 | Light, reduced transparency off (`matchMedia` in Web Inspector). Sidebar and Quick Panel were visually inspected before the final tint adjustment. Full Quick Panel had no unexplained blank region. |

The QA bundle used `com.zenith.desktop.qa`, allowing inspection alongside the
installed 0.3.56 app. The native layer showed AppKit `Sidebar`/`Popover`
materials and a transparent WebView backing. Against the dark desktop, the
initial 28–30% light tint made chrome too gray and reduced label contrast;
the final CSS tint is 62% light / 55% dark. A second native capture after this
last adjustment was unavailable because the computer-use service stopped
enumerating all app windows (`cgWindowNotFound`). The final native appearance,
dark mode, reduced-transparency fallback, and other backdrop colors need
review in the PR before merging.

## Destination review

| Destination | Finding and action |
| --- | --- |
| Overview | At 800 × 560 the cleanup summary and CPU, memory, and battery remain readable. The decorative planet remains separate from health status. Stale/failed inventory now says so instead of displaying an unverified `0 B`; working phases share the Quick Panel's three-dot status. Pinned sidebar brand prevents it scrolling away. |
| Storage Cleanup and category detail | The header now uses the shared page pattern. The scan button appears only on Cleanup. Scan and post-clean verification show a compact phase, count, and optional path detail; obsolete categories and selection controls stay hidden until verification completes. The category detail route and back action were inspected in preview. |
| Storage Developer Artifacts | Reviewed at 800 × 560. Existing workspace and scan actions remain in the subview; no new overflow observed. Destructive review was not executed. |
| Storage Large Files | Reviewed at 800 × 560. File inspection and action hierarchy remain in the subview; no new overflow observed. No real files were trashed. |
| Storage Applications | Reviewed at 800 × 560. App inventory and review hierarchy remain in the subview; no new overflow observed. No real app was removed. |
| Storage Disks | Reviewed at 800 × 560. Disk readings stay in their dedicated view; no new overflow observed. |
| Performance CPU, Memory, Battery | Each detail tab was inspected at 800 × 560. Memory's usage bar now uses the same cobalt data accent as other generic metrics; pressure retains separate semantic wording. |
| AI Activity usage, projects, adapters | Usage/projects/adapters were inspected at 800 × 560. The route now uses the shared page header and the adapter cards stack when the content column is narrow; the legacy-marker action no longer crowds a status row. |
| AI Control Center | Inspected at 800 × 560. Shared header and tab component replace the compressed custom header and tab row; tabs expose tablist, tab, and tabpanel semantics. |
| Containers | Reviewed empty/stopped browser state at 800 × 560; no layout change needed. |
| Local Models | Model rows now stack on narrow content widths so badges, size, and actions do not collide. |
| Dev Servers | Listener rows now stack at narrow widths; long labels and action columns stay separate. |
| Keep Awake | Heading and inactive wording match navigation; rule controls were inspected at 800 × 560. |
| Settings | Settings stays pinned above the footer when navigation scrolls. Preferences and appearance controls remained reachable at 800 × 560. |
| Quick Panel, sidebar, dialogs | Quick Panel was inspected at 320/360/400 browser widths and 400 native width. Scan feedback uses [one compact status and three dots](quick-working-browser.png), and `Review` changes to `View scan` while working. [320 px](quick-320-browser.png) / [400 px](quick-400-browser.png) previews show the footer and rows without clipping. The selected underline tab keeps a single visible keyboard outline. Result/error dialogs and destructive confirmations were reviewed by code and regression tests; no real cleanup was performed. |

## Verification and remaining native review

- `cargo check`, `cargo test`, `pnpm check`, `pnpm test -- --run` (394 tests), `pnpm build`, and final `just build-fast` passed. The first concurrent `cargo test` run exited during the build; the sequential rerun passed.
- Frontend tests cover Quick Panel sizing permission and minimum, shared cleanup phases, compact working state, absence of stale category rows, and Storage post-clean refresh behavior.
- Native screenshot comparison over contrasting backdrops, 800 × 560 native resizing, sparse Quick Panel/reopen/display switching, dark and reduced-transparency visual checks, and keyboard-only native pass remain for PR review. The browser images above are not evidence for those native conditions.
- Real cleanup and destructive confirmation were not exercised against user data. Backend authorization and cancellation coverage remained in the passing Rust suite.
