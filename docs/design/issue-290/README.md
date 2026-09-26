# Issue 290 — glass and resource review

## Changes

- macOS 26+: public `NSGlassEffectView` with the WKWebView as its content view.
  Runtime class lookup retains Sidebar/Popover vibrancy on older macOS.
- Synchronize native appearance with the saved Light / Dark / System preference.
  Light glass uses an 18% tint; dark glass uses 55% for supporting-text contrast.
- Developer artifact rows use neutral partial-measurement badges and supporting
  text. Action-level amber warnings and explicit partial-cleanup consent remain.
- Quick Panel uses consistent resource icons and a compact storage review row.
  The final feedback pass removes the bright cleanup border, leading stripe,
  uppercase label, and oversized nonnumeric status. Working states keep the
  three-dot indicator.
- Overview exposes CPU, memory, and disk together, with a resource-review
  disclosure containing the storage shortcut and existing application actions.
  The reused MemoryPanel omits duplicate gauges in compact mode. The quit
  confirmation uses a native HTML dialog for focus trapping and Escape.
- Overview owns a visible-only subscription to the existing memory collector.

## Verification — September 26, 2026

Application version: 0.3.58. Host: macOS 27.0 (26A428).

Passed: `cargo check`, `cargo test`, `pnpm check` (zero errors/warnings),
`pnpm test -- --run` (394 tests, 41 files), `pnpm build`, `just build-fast`,
`just check-architecture`, `just check-version`, and `git diff --check`.
The copy assertions in the existing Quick Panel test were updated for the
shorter working-state labels.

### Native inspection

An isolated `Zenith QA.app` (identifier `com.zenith.desktop.qa`) was built and
opened, leaving the installed application intact. Main-window and Quick Panel
rendering were inspected. Light/Dark switching updated the native appearance.
Web Inspector reported `native-liquid-glass = true`, reduced transparency off,
and the expected 18% light sidebar tint. The gray light-theme mismatch was no
longer visible after native-theme synchronization.

The final cleanup-row simplification was inspected in the browser preview and
included in the standalone app build. Older macOS fallback, Windows rendering,
all desktop-background combinations, and native reduced-transparency switching
were not exercised here. Native window capture became intermittently unavailable,
so the PNG files below are browser fixtures, not evidence of desktop translucency.
An existing injected notification-permission rejection appeared in Web Inspector;
this change does not expand notification permissions.

### Browser inspection

Overview was inspected at 800×560, 960×660, and 1440×900; Quick Panel at 400×640,
including dark/reduced-motion rendering. Resource review exposes the shared app
list. Opening Quit displays only Cancel and Quit Normally; Escape dismisses the
modal without sending a termination request. Partial artifact selection retains
its explicit warning/consent flow. No real cleanup or application termination
was performed.

- `overview-light.png`, `overview-small.png`: Overview layout (earlier 0.3.57
  development asset stamp; same revised layout).
- `resource-review-small.png`: compact running-app review.
- `artifacts-light.png`: restrained partial-measurement row.
- `quick-dark-final.png`: final compact cleanup treatment, using mock data and
  a forced dark class for visual inspection.

## API references

- [Apple NSGlassEffectView](https://developer.apple.com/documentation/appkit/nsglasseffectview)
- [Apple contentView contract](https://developer.apple.com/documentation/appkit/nsglasseffectview/contentview)

CPU and memory are not advertised as automatically cleanable caches. Closing
apps always goes through the existing protected, lease-backed confirmation flow.

## 0.3.59 follow-up — September 26, 2026

- Cleanup now shows its label, one short state/value, and one contextual action.
  Repeated explanatory paragraphs and the second action row are removed. Full
  reasons remain in native tooltips and Storage. See `quick-0.3.59.png` (browser
  fixture, 400×620, dark and reduced motion).
- Quick Panel battery icons distinguish external power from active charging.
  During investigation, macOS `pmset -g batt` itself reported 91%, AC attached,
  not charging. Zenith agreed; no backend state was overridden or charging-stop
  cause inferred.
- `just bump-patch` advanced all manifests to 0.3.59. README now describes the
  current Overview, adaptive panel, native material, provider network behavior,
  workspace safety-test locations, patch commands, and signature examples.
