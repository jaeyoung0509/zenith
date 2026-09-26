# Zenith Design System

Zenith is a compact, native-feeling macOS and desktop utility for developers.
Its visual direction pairs cool near-white surfaces and ink-blue type with
pastel periwinkle actions and cobalt navigation. Frosted translucency gives navigation and
the Quick Panel a native sense of depth. Technical data and cleanup decisions
stay on legible solid surfaces; amber, red, and green remain semantic signals
for caution, failure, and observed success. The dark theme follows the same
cool-neutral and periwinkle relationship.

Celestial atmosphere gives the interface a distinct identity: quiet cobalt and
ice-blue light on the shell, a small original planet with orbital dust in the
Overview summary, and a soft lavender highlight. The planet is purely
decorative, never a machine-health indicator. Its local SVG cloud layer rotates
over 36 seconds and the orbital dust over 48 seconds. CSS animates transforms
only; an IntersectionObserver and document visibility pause the scene when
hidden. Reduced motion and reduced transparency stop it. No telemetry polling
is added for this decoration.

This document is the visual contract. The executable half of it — colour,
radius, type, and motion tokens — lives in `src/app.css` and is enforced by
`src/test/designSystem.test.ts`.

## Product character

The material reference is the macOS Bluetooth popover supplied during the
September 25, 2026 design review: one rounded translucent surface, fine internal
dividers, and crisp text. Use this material for transient chrome and navigation;
keep operational lists on solid working surfaces. The UI UX Pro Max glassmorphism
and accessibility guidance informs this treatment; existing Zenith tokens and
desktop sizing remain the source of truth.

On macOS 26 and later, an `NSGlassEffectView` using the public `Regular`
style hosts the WKWebView as its `contentView`. This provides native Liquid
Glass behind the sidebar and Quick Panel; the main working surface stays
opaque. The native appearance follows the saved Light / Dark / System setting,
so a light interface never intentionally sits on a dark AppKit material.
A light 18% / dark 55% CSS tint adds the pastel tone. The Quick Panel uses a
20 px native corner radius. Runtime class detection retains the existing
`Sidebar` / `Popover` vibrancy with 62% / 55% tint on older macOS. The two
native materials are never stacked. Browser previews cannot demonstrate desktop
translucency. Windows retains its opaque adapter. Reduced transparency makes
chrome opaque; native visual QA must inspect actual composited readability.

- Native desktop developer utility, not a marketing dashboard.
- Calm, technical, trustworthy. Ink blue carries reading text; pastel
  periwinkle identifies primary actions, with cobalt for selected navigation.
  One obvious primary action per task. Generic resource readings use cobalt;
  green is reserved for a completed success or cleanup eligibility.
- The cobalt split `Z` is the product identity: two substantial diagonal
  segments separated by one clear horizontal cut. Use the compact light tile
  in app chrome, the full app tile for Finder/Dock, and the monochrome template
  only for the macOS menu bar. The mark has no status dot or badge.
- A logo is identity, not a trust certificate. No screen claims a machine is
  healthy, safe, or protected in general terms.

## Foundation and semantic tokens

The canonical colour, typography, radius, and motion tokens live in
`src/app.css`, using Tailwind CSS 4's CSS-first `@theme` configuration.
Tailwind's Vite plugin is the only build integration; there is no legacy
JavaScript theme configuration. The root app targets the Tailwind 4 WebView
baseline (Safari 16.4+, Chrome 111+, or Firefox 128+) and keeps the onboarding
package on its own independent Vite pipeline. `pnpm build` verifies the
generated CSS contains the utilities the redesign depends on before a bundle is
considered valid.

### Colour and surface hierarchy

Tokens are stored as bare HSL triplets so both themes can be swapped without a
rebuild. Measured contrast (WCAG 2.1, sRGB) is recorded per pair; normal text
targets ≥ 4.5:1 and meaningful controls, boundaries, and focus target ≥ 3:1.

| Token | Light | Dark | Role |
| --- | --- | --- | --- |
| `--background` | `230 35% 97%` | `222 24% 8%` | Cool window / page |
| `--card` | `230 40% 99%` | `222 20% 13%` | Solid working surface |
| `--secondary` | `230 35% 94%` | `222 20% 12%` | Sidebar and grouped surface |
| `--accent` | `228 75% 93%` | `219 42% 24%` | Selected / quiet blue surface |
| `--border` | `230 24% 87%` | `220 18% 25%` | Decorative hairline separator |
| `--border-strong` | `225 13% 49%` | `220 20% 49%` | Essential boundary (inputs, interactive chrome) |
| `--foreground` | `225 24% 17%` | `230 35% 95%` | Ink-blue primary text |
| `--muted-foreground` | `225 13% 40%` | `220 14% 74%` | Supporting text |
| `--primary` | `220 76% 38%` | `218 82% 76%` | Cobalt text / data accent |
| `--action` | `225 86% 87%` | `225 66% 80%` | Pastel primary button |
| `--action-foreground` | `225 48% 24%` | `225 48% 15%` | Text on pastel buttons |
| `--primary-foreground` | `0 0% 100%` | `224 48% 13%` | Text on saturated cobalt surfaces |
| `--ring` | `220 82% 36%` | `217 88% 68%` | Focus outline |
| `--success` | `162 71% 24%` | `157 60% 29%` | Safe candidates, protected items, healthy readings |
| `--warning` | `33 100% 27%` | `38 76% 63%` | Rebuild caches, cautionary states, elevated pressure |
| `--destructive` | `3 71% 41%` | `4 75% 70%` | Destructive actions, kills, hard errors |
| `--ai` | `222 82% 37%` | `217 85% 72%` | AI metadata accent, shared cobalt family |

Measured contrast is enforced from the executable tokens in
`src/test/designSystem.test.ts`. Normal text targets ≥ 4.5:1 and meaningful
controls, boundaries, and focus target ≥ 3:1 in both themes. Keep supporting
copy readable on both the cool canvas and white working surfaces; do not rely
on a previous palette's contrast measurements when changing a token.

The dark palette is checked by the design-system contrast test whenever its
tokens change. Translucent chrome falls back to an opaque surface when the
system requests reduced transparency.

Decorative `--cosmic-*` tokens are separate from semantic status colours.
Reduced transparency removes atmospheric backgrounds and the planet. Narrow
content areas hide the artwork before it can crowd text or actions. Cards and
data tables retain solid surfaces in both themes.

The cobalt mapping deliberately uses a pale blue selected surface and pastel
periwinkle action in light mode. Dark mode retains a light action surface and
the same hue family. This keeps action text legible and selected state distinct
from both the page and the action. Brand colour is never the only carrier of
meaning: every selected, safe, or cautionary state also carries text, an icon,
or a shape. Success remains green because it names an observed outcome, not the
brand.

### Keyboard focus

`--ring` is the focus token in each theme, applied through
`focus-visible:ring-2 focus-visible:ring-ring` (or the `.focus-ring` outline
helper). A focus ring is never removed without a visible replacement. Measure
it against the surface immediately outside each focused control, including
offset rings, rather than assuming every control sits on the page background.

### Typography scale

- System UI font with Korean fallbacks for labels, navigation, and prose;
  monospace only for paths, ports, byte counts, percentages, and identifiers.
- Tabular numerals (`font-mono tabular-nums`) wherever a value can change in
  place.
- Page headings: `text-title` (22 px). Key metrics: `text-metric` (28 px) or
  `text-metric-lg` (32 px) for a single headline metric per page.
- Body and controls: `text-body` (13 px) / `text-sm` (14 px).
- Supporting information: `text-meta` (12 px). Captions and timestamps:
  `text-caption` (11 px). Micro badges and status dots: `text-micro` (10 px).
- A critical warning never uses `text-micro`, and "9 px" warnings are not
  allowed anywhere. Arbitrary `text-[Npx]` utilities are rejected by the design
  test; use the named steps.

### Spacing, geometry, and density

- 4 px spacing scale: 4 / 8 / 12 / 16 / 24 / 32 px.
- Radii: 10–12 px controls (`rounded-md`/`rounded-lg`), 16 px grouped surfaces
  and rows inside a surface (`rounded-xl`), 20 px overlays and the Quick Panel
  shell (`rounded-2xl`). Neither hard squares nor pill-shaped everything.
- Main window: 960 × 660 default, 800 × 560 minimum, expanded by users on
  larger displays. Sidebar 224 px expanded, 64 px collapsed.
- Content insets are driven by the **content width**, not the viewport: 16 px
  until the content column passes 48 rem, 24 px above it. `<main>` is a
  container (`@container`) so a 960 px window and a 1440 px window inset the
  column by the same rule when the sidebar is expanded.
- Resource rows 52–64 px; compact rows 40–44 px; main controls 32–36 px high;
  icon-only targets at least 28 × 28 px, preferably 32 × 32 px.
- Separate a flexible name from a fixed-width value or action column. Keep a
  value and its unit together, wrap long names between words, and let technical
  paths truncate into a readable detail surface instead of shrinking every
  label.
- Group comparable rows inside one surface. Never nest a card around every
  line. The Overview focal surface may use one soft shadow; dense rows stay
  flat so the glass layer remains distinct.

### Native window corners

The rounded main-window silhouette is owned by macOS. Zenith keeps the real
overlay title-bar controls, drag region, resizing, full-screen behaviour, and
the OS shadow. On macOS its WebView backing is transparent so native vibrancy
shows through the sidebar; the main content paints an opaque surface. CSS
`border-radius` does not round the OS window. The Quick Panel keeps its transparent, undecorated backing and
clips its own 20 px radius, which is what the shadow and hit testing follow.
Do not simulate traffic lights in CSS and do not add private window hacks to
chase a mockup radius.

## Motion specification

Functional motion communicates state change; it never delays input, replays a
list, or animates a number continuously. The user-requested celestial scene is
the single decorative animation, subject to visibility and motion preferences.

### Material and emphasis

- New installations open in the light theme. A saved theme choice is retained.
- The sidebar and Quick Panel header/footer may blur the content behind them.
  The Overview cleanup focal surface is a solid near-white surface with a
  restrained cobalt edge.
  File lists, category rows, dialogs, warnings, and selected cleanup targets
  also use solid surfaces.
- Keep gradients, decorative rings, inflated type, and repeated floating cards
  out of operational screens. Reduced transparency substitutes an opaque
  chrome surface, and reduced motion removes decorative movement.
- Give each operational surface one focal point: a primary action, meaningful
  state, or key measured value. Supporting metadata stays quieter, and
  comparable values align to one column.
- Use surface luminance and blue/lavender accents and surface grouping to add character before adding
  decoration. Reserve saturated cobalt for actions, selection, focus, and a
  small number of identity cues.
- The cleanup workflow has one visible phase at a time: reviewed selection,
  execution, then inventory refresh. While refreshing after a completed clean,
  hide the old selection toolbar and category list. The result dialog opens
  after the new inventory is measured. Overview and Quick Panel use the same
  cleanup phase projection: stale, failed, unknown, and partial inventories
  never render an unverified zero as a confirmed cleanable total.

| Interaction | Duration & easing | Behaviour |
| --- | --- | --- |
| Hover / focus / pressed | 100–140 ms (`--duration-instant`, `--duration-fast`) | Colour, border, or opacity only; dense rows never move |
| Navigation / content change | 140–180 ms (`--duration-normal`) | Opacity plus at most 2–4 px translation; route changes never queue |
| Dialog / sheet | 160–220 ms (`--duration-overlay`) | Opacity-led; focus is trapped on open and returned on close |
| Value update | Immediate | The changed value is replaced in place; no counter roll, no list replay |
| Sidebar collapse | ≤ 180 ms | Width and padding only, labels fade; content stays fluid |
| Observed working state | 2–3 s breathing dot | Only for an actually observed, currently running agent session while the surface is visible |

`prefers-reduced-motion: reduce` removes decorative movement: breathing,
spinners, and every transition are disabled while value updates and focus
feedback remain. Nothing animates in a hidden window.

## Layout and information architecture

### Navigation

One primary navigation system, grouped, with Tools expanded by default:

```text
Overview          the control tower; the default start page for new installs
Storage           Cleanup / Developer Artifacts / Large Files / Applications / Disks
Performance       CPU / Memory / Battery detail
AI Activity       Usage / Projects & Sessions / Tool Adapters / AI Control Center

Tools             visual group, expanded by default
  Containers
  Local Models
  Dev Servers
  Keep Awake

Settings          anchored at the bottom
```

- `Overview` is the user-facing name of the control tower. It summarizes the
  existing services and links to them; it is not a second policy layer and it
  never mounts a full route offscreen to collect data.
- The Memory route is `Performance → Memory`. The persisted `memory` tab id and
  the `memory` route both resolve to that page, so saved layouts and deep links
  keep working; `#280`'s migration adds `Overview` **after** whichever tab the
  user starts on, so an upgrade never changes the start page.
- Group headings name a navigation group once. Saved tab order remains
  authoritative, including when destinations from one group become separated;
  never repeat a heading just because a group resumes later in the list.
- Local tab strips belong to a page's own sections. There is no second global
  Overview/Cleanup/Performance/AI strip inside a page.

### Standard page pattern

**Page heading + one contextual action → useful summary → filter/list →
details when requested.** Not every screen is a row of equal-weight metric
cards; a page that has one task shows one task.

Summary cards are navigation, not concealed destructive buttons. A summary
click opens the exact destination with its relevant filter or selection
context, and returning restores the list, filter, and scroll position while the
inventory is still valid.

## Components and interaction contract

- **Button**: semantic variants (`primary`, `secondary`, `outline`, `ghost`,
  `destructive`) and sizes (`xs`, `sm`, `md`, `lg`, `icon`) with
  `focus-visible:ring-2 focus-visible:ring-ring`, a visible disabled state, and
  a tooltip or explanation when disabled.
- **Card / surface**: `bg-card` + 1 px `--border` + `rounded-xl`. Data rows
  remain opaque; the optional `surface="subtle"` variant groups rows on
  `--secondary`. Navigation and Quick Panel chrome carry translucency.
- **Navigation group**: each group heading appears once, even when a saved tab
  order separates destinations from the same group. Selected navigation uses
  a pale blue surface, cobalt text, and a visible leading marker.
- **Badge**: status carries text meaning beyond colour (`success`, `warning`,
  `outline`), at `text-caption` or above.
- **ProgressBar**: 4–8 px tall, no looping shimmer, no indeterminate gradient.
- **ByteValue**: monospace tabular numerals for every byte metric.
- **SelectionToolbar**: reserved space in the content column; count, measured
  bytes, and risk summary, with one primary action.
- **SegmentedTabs**: `role="tablist"` with one `tabindex="0"`, `aria-selected`
  on the active tab, and `aria-controls` pointing at the panel; the selected
  tab uses the pale blue `--accent` surface rather than a dark pill.
- **Switch / Checkbox**: switches are for persistent on/off state only,
  checkboxes for selection. The enabled switch track and the selected checkbox
  use `--success` with a white check; never a plain white track.
- **EmptyState**: separate states for empty search results, no inventory,
  missing platform capability, and failed loading.
- **InlineNotice**: `role="status"` for information, `role="alert"` for errors,
  with the concrete reason and, when the action can be retried, one retry.

### Brand identity

- Zenith's own mark is authored once in `src-tauri/icons/zenith-mark.svg`.
  `pnpm icons:generate` derives every packaged size, native `.icns` / `.ico`,
  the menu-bar PNG, public assets, compact frontend SVG, and its registry hash.
  `pnpm icons:check` detects drift. Do not hand-edit generated copies or recreate
  the Z with a font, emoji, or another icon library.
- The mark uses cobalt `#1748AB` on a cool light tile. Compact artwork has no
  external shadow or Dock padding, so the 20 px Quick Panel and 24 px sidebar
  uses remain readable. The native app tile has its own padding, restrained
  relief, and the same geometry. Keep the central cut open at small sizes.
- macOS uses a black-alpha 44 px template PNG for its 22 pt menu-bar surface;
  AppKit owns the light/dark appearance. Windows/Linux use the full-color tray
  icon, never a black template. These logo variants do not change window glass
  materials or their tint. Use the same compact tile in light and dark themes.
- `BrandIcon` resolves an identity through one typed registry: a reviewed local
  asset when the copyright holder's licence clearly permits redistribution, and
  otherwise a neutral two-letter monogram beside the factual product name.
  Provider lists in the Quick Panel and AI usage view use names without identity
  icons, so mixed asset availability never changes their hierarchy.
- Reviewed assets live in `src/lib/assets/brands/`, are served offline from the
  bundle, keep their intrinsic proportions and colours, and are never filtered,
  recoloured, distorted, or clipped. The asset register with source URL,
  revision, licence, and notices is `docs/design/brand-assets.md`.
- A brand mark is decorative next to its name (`alt=""` / `aria-hidden`); the
  adjacent text carries the accessible name.
- Tool rows use 18–20 px action glyphs and a 32 px identity slot (24 px
  artwork) so logos sit next to useful names rather than in a wall of
  promotional cards.
- The Quick Panel and main sidebar share the same Zenith asset and restrained
  functional navigation icons. Provider rows never borrow sparkle or rocket
  symbols as substitute logos.

## Feature-specific patterns

### Overview (control tower)

Compact heading, a cleanup summary with `Review resources` and a storage review
action, CPU / memory / disk tiles showing real measurements, a compact battery
reading, a
compact list of active tools and services, and a compact Keep Awake control. At
960 × 660 the next action and the core metrics are visible without scrolling;
secondary detail scrolls. Each tile opens its exact detail page.

`Review resources` expands a storage review shortcut and the existing running-app
list in place. The compact list omits duplicate memory gauges. Application quit
uses the same lease-backed confirmation and graceful-before-force flow as
Performance. It is never advertised as an automatic CPU or RAM cleanup.
Overview subscribes to the shared memory collector only while visible.

### Quick Panel

- At 320–400 px, use compact full-width reading rows. A label, current value,
  and one supporting fact should fit in roughly 48 px; details stay in the
  main window. The header and footer stay fixed while the body scrolls.
- The panel height follows its visible content between a 300 px minimum and a
  740 px maximum, capped by the active display work area. Recompute only when
  section content changes, not on telemetry value updates; preserve tray
  alignment when reopening the persistent window.
- Cleanup is the lead actionable summary. A missing or stale scan shows an
  explicit scan-needed state and `Scan Again`; it never presents stale bytes
  as verified cleanable space. A fresh non-zero estimate may use the focal
  value treatment. Scanning, cleanup, and post-clean verification use the
  three-dot working indicator beside one short status sentence. Hide stale
  category rows and review actions until the new inventory is ready.
- A complete scan may offer one-click `Clean Safe`. A partial scan with measured
  Safe items opens a compact review inside the Quick Panel; the user chooses
  exact items and confirms there. Unknown locations never contribute bytes or
  authorization. Show the measured cleanup result immediately while the
  follow-up scan checks what remains.
- Nonnumeric scan states use compact system text, never the large numeric
  byte style. The cleanup summary uses a compact icon, stable `Cleanup` label,
  small status or byte value, and exactly one contextual action. Detailed
  reasons are available in the tooltip and Storage, not a repeated paragraph. It has no leading
  warning stripe or bright enclosing border; resource icons
  share one pastel treatment.
- Resource values align consistently and carry more visual weight than their
  labels and metadata. Use a compact proportional indicator only when it
  represents a measured quantity; memory pressure remains separate from used
  memory.
- Show each AI provider once. Merge observed sessions into its identity row,
  use a readable name and an explicit loading/stale/unavailable state, and
  express reset times with units rather than a bare minute counter. AI activity
  uses typography and grouping for identity, with no separate purple brand.
- Loading provider usage uses the shared three-dot indicator. Stale usage has
  a labeled refresh action in place. The battery row uses the same filled
  indicator as the dashboard, including a bolt only for actual charging.
- On macOS the Quick Panel uses native Popover vibrancy behind a lightly tinted
  WebView surface. The dashboard may use Liquid Glass separately; the quick
  surface must still reveal the desktop behind it. Respect Reduce Transparency.
- Cleanup and Keep Awake keep their actions within the same flat row hierarchy.
  The storage safety and freshness rules do not change with the presentation.
- Memory leads with the measured used amount. Pressure is a smaller explicit
  label (`Low pressure`, `Elevated pressure`, or `Critical pressure`), not a
  large monospace verdict. Overview follows the same hierarchy and displays
  total capacity, swap when present, and a usage bar separately from pressure.

### Storage

- Cleanup reads in task order: scan summary, freshness, category selection,
  then review. The primary figure is the cleanup estimate; observed bytes use
  a smaller secondary figure and respect ambiguous overlap ranges. Selected
  bytes belong to the review toolbar, not the estimate.
- Category rows share one bordered surface, with aligned value columns and
  descending cleanable-byte order. Local workflow tabs use an underline rather
  than a second enclosing card. The review toolbar follows the list in keyboard
  order and remains visible while scrolling.

- Top summary separates disk capacity, observed store size, the known cleanup
  estimate, and the selected amount. Unknown prune size can still be reviewed
  and cleaned.
- `Cleanup`: category rows with risk tiers, an actionable selection footer, and
  the existing backend eligibility, consent, and one-shot plan rules.
- `Developer Artifacts`: workspace/project list with generated-directory
  amounts aligned in a fixed value column, workspace authorization, and its
  own inventory freshness. Comparable projects share one list surface. Keep a
  warning reason visible in the row, with a neutral `Partial measurement` badge
  for incomplete measurements. Reserve amber for action-level caution and
  explicit partial-cleanup consent, rather than tinting each project row. Keep evidence and rebuild guidance in an
  accessible disclosure. No automatic whole-home scan and no unreviewed
  project deletion.
- `Large Files`, `Applications`, `Disks`: searchable size-sorted files,
  installed-app rows with related data, and volume capacity. No invented
  disk-map engine.

### Performance

- CPU, memory, and battery summaries with local tabs and a short real history
  when samples exist.
- Memory pressure is shown first and is distinct from percent used; supporting
  facts are used/total and swap.
- Battery is a small 2D outline with a proportional solid fill: percentage plus
  the actual charging state. Lightning appears only while actually charging —
  AC power alone is not charging. Time remaining appears only when the platform
  returns a meaningful estimate. A desktop without a battery omits the row.
- Warm-up, stale, unavailable, and failed readings are stated instead of being
  drawn as zero.
- Graceful application actions come before force, through the existing backend
  routes and allowlists.

### AI Activity

Provider usage, account quota, token/cost readings, disk storage, and observed
process activity stay separate; incomparable provider quotas are never summed.
Session identity comes from the observed adapter/session state, so a parent
editor's logo is never used as proof of a specific agent. Unknown and
unavailable states are named explicitly.

### Containers (Docker)

Missing runtime, stopped daemon, and load failure are different states.
Volumes are stateful resources and are not generic cache; the reviewed owner
actions stay, with no extra global cleanup shortcut.

### Local models

Model weights are not cleanup junk. Provider grouping, model/revision identity,
storage amount, and the supported per-item actions are preserved, including
typed deletion where the backend supports it.

### Development servers

Port, project/tool identity, observed address and exposure come first, then the
resource and action menu. A listening process is not a server Zenith started,
and release/termination keeps its verified flow.

### Keep Awake

Current session, duration, source, start/stop, readable rules, and the native
application picker. Manual and rule priorities are preserved and the state
changes only after the backend confirms it.

### Settings

Focused sections for appearance, navigation and Quick Panel, cleanup,
providers, notifications, and diagnostics. No telemetry opt-in, charge limit,
safety buffer, or other invented setting.

## Data policy for system readings

- One collector per fact, shared by Overview, Performance, and the Quick Panel
  through subscriber-owned stores. A reading stops when its last visible
  subscriber leaves; a hidden window never polls or redraws.
- **CPU**: a defined system-wide 0–100% normalization across all logical cores,
  sampled no faster than the platform's minimum CPU update interval. Warm-up
  (fewer than two readings), stale (older than the freshness budget),
  unavailable (no adapter), and failed (probe error) are explicit states; a
  percentage is only ever reported from a real pair of readings.
- **Memory**: pressure first, used/total and swap as supporting facts. Health is
  never inferred from a single usage ratio and incompatible categories are
  never summed.
- **Battery**: percentage plus the actual charge state
  (charging / discharging / full / plugged-in-not-charging / unknown). Absent
  and unavailable are different from zero.
- **Activity**: current observed adapter and session states only. A resident
  process is not proof of inference, and prompts, edited files, active hours, or
  model generations are never invented.

## Accessibility and visual QA

- WCAG AA contrast: normal text ≥ 4.5:1, meaningful controls, boundaries, and
  focus ≥ 3:1 in both themes. Decorative separators are not the only boundary
  between two interactive regions.
- Full keyboard traversal with visible focus; dialogs trap focus and return it
  to the invoking control.
- Accessible names, tooltips, and labels on every icon-only button.
- No information is conveyed by colour alone, and long English/Korean labels
  wrap between words instead of shrinking or breaking inside a word.
- Every new screen is checked at 800 × 560, 960 × 660, and a larger desktop
  size, in light and dark themes, with reduced motion enabled.
