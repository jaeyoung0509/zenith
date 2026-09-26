# Issue 304 — restore the approved Quick Panel glass

## Version comparison and cause

The user supplied a native v0.3.59 screenshot on September 26, 2026 as the
accepted visual reference. Desktop colors and broad light/dark regions show
through its frosted Quick Panel; text and controls remain crisp.

| Setting | v0.3.59 (`a34783e`, #291) | v0.3.61 (`61283ac`, #303) | This change |
| --- | --- | --- | --- |
| Main, glass API available | Liquid Glass Regular | Same | Same |
| Quick, glass API available | Liquid Glass Regular | Popover vibrancy | Restore Liquid Glass Regular |
| Light Quick tint on newer macOS | 18% | 32% | 18% |
| Dark Quick tint on newer macOS | 55% | 38% | 55% |
| Older-macOS Quick tint, light / dark | 62% / 55% | 32% / 38% | 62% / 55% |
| Main sidebar gradients and native host | Present | Same | Same |

#303 disabled the glass host, CSS marker, and vibrancy removal for `quick` by
adding a `label == "main"` restriction to each branch. This is the material
selection regression relative to the accepted version. The earlier suggestion
to remove Liquid Glass was inconsistent with the v0.3.59 source and is corrected
here. Main-window appearance differences in screenshots with different desktop
backgrounds do not establish a main-window code regression.

Window creation now consumes one tested material decision for effect removal,
CSS marker injection, and native host installation. The older test covered only
`platform_window_config`, before the final material override, so it could pass
while the actual glass selection regressed. New Rust tests cover both labels
with and without the glass API; the CSS contract test covers both native tints
and the final solid-surface override.

The protected contract in [DESIGN.md](../../../DESIGN.md#protected-native-glass-contract--user-approved-v0359)
requires explicit user direction before a future material redesign. Routine UI
polish is not authorization to replace the material or alter these opacities.

## Verification — September 26, 2026

Application version: 0.3.61, issue-304 working tree based on `61283ac`.
Host: macOS 27.0 (26A428).

Passed:

- `cargo check --workspace`
- `cargo test --workspace` (including both new material tests and doc tests)
- `pnpm check` (zero errors and warnings)
- `pnpm test -- --run` (395 tests across 41 files)
- `pnpm build`
- `just build-fast` (standalone debug `.app` with embedded frontend)
- `cargo clippy --workspace --all-targets -- -D warnings`
- `just check-architecture`, `cargo fmt --all -- --check`, `just check-version`
- `git diff --check`

### Native runtime inspection

An isolated `Zenith Glass QA.app` (`com.zenith.desktop.glass-qa`) was built from
the same source/frontend and launched alongside the installed application.
The installed app and its active Keep Awake session were left running. The QA
app was closed after inspection.

Web Inspector in the actual native windows reported:

| Observation | Main sidebar | Quick Panel |
| --- | --- | --- |
| `native-liquid-glass` | true | true |
| Computed light tint | `rgba(244, 245, 250, 0.18)` | `rgba(244, 245, 250, 0.18)` |
| CSS backdrop filter | `none` | `none` |
| Reduce Transparency media query | false | false |

Quick Panel body backing was `rgba(0, 0, 0, 0)`. Its content and rounded shell
rendered, and the current cleanup/provider UI remained present. These are
runtime configuration checks, not a claim of complete screenshot equivalence.
An existing notification-permission rejection also appeared in the inspector;
notification permissions are outside this change.

Controlled desktop-composited comparisons over matching colorful and dark
backdrops, native dark/Reduce Transparency switching, older macOS, and Windows
were not exercised in this pass. The source comparison and runtime inspection
verify restoration of the approved material path; those remaining visual cases
must not be reported as passed.
