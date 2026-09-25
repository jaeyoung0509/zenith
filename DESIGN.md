# Zenith Design System

Zenith is a compact, native-feeling macOS and desktop utility for developers.
The visual direction starts with a white working surface. Mint identifies
selection and action, while restrained translucency gives navigation and the
Quick Panel a native sense of depth. Technical data and cleanup decisions stay
on legible solid surfaces; amber and red signal real caution and failure. The
dark theme remains an option, not the primary visual reference.

This document is the visual contract. The executable half of it — colour,
radius, type, and motion tokens — lives in `src/app.css` and is enforced by
`src/test/designSystem.test.ts`.

## Product character

- Native desktop developer utility, not a marketing dashboard.
- Bright, technical, trustworthy. One obvious primary action per task.
- The white circular `Z` mark is the product identity. Use the template-style
  monochrome variant for the menu bar and the full app icon for Finder, Dock,
  title areas, and application menus.
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
| `--background` | `120 11% 98%` | `220 25% 8%` | Window / page |
| `--card` | `0 0% 100%` | `217 19% 13%` | Solid content surface |
| `--secondary` | `140 16% 96%` | `219 21% 11%` | Sidebar and subtle surface |
| `--accent` | `147 31% 93%` | `169 39% 20%` | Selected / quiet mint |
| `--border` | `137 13% 90%` | `194 18% 23%` | Decorative hairline separator |
| `--border-strong` | `145 10% 52%` | `166 17% 49%` | Essential boundary (inputs, interactive chrome) |
| `--foreground` | `153 19% 11%` | `162 23% 95%` | Primary text |
| `--muted-foreground` | `148 6% 40%` | `174 10% 72%` | Supporting text |
| `--primary` | `161 42% 18%` | `154 43% 76%` | Primary action surface |
| `--primary-foreground` | `0 0% 100%` | `159 43% 9%` | Text on the primary action |
| `--ring` | `160 45% 42%` | `164 45% 34%` | Focus outline |
| `--success` | `162 71% 24%` | `157 60% 29%` | Safe candidates, protected items, healthy readings |
| `--warning` | `33 100% 27%` | `38 76% 63%` | Rebuild caches, cautionary states, elevated pressure |
| `--destructive` | `3 71% 41%` | `4 75% 70%` | Destructive actions, kills, hard errors |
| `--ai` | `256 51% 47%` | `258 82% 79%` | AI Activity identity, models, provider metadata |

Measured light-theme contrast: primary text 16.5:1 on white and 15.8:1 on the
page; supporting text 5.3:1 on white; white on the primary action 10.2:1;
focus `--ring` 3.3:1 on white, 3.1:1 on the page, 3.1:1 on the primary action,
3.0:1 on the sidebar; essential boundaries 3.4:1 on white; status colours
6.6–7.7:1 on white.

The dark palette is checked by the design-system contrast test whenever its
tokens change. Translucent chrome falls back to an opaque surface when the
system requests reduced transparency.

Two deliberate deviations from a naive mint mapping are recorded here because
they are load-bearing:

- The dark focus ring is a medium mint rather than a light one. A single token
  has to clear 3:1 against both the charcoal page and the light-mint primary
  action surface; a light ring would fail against the primary action.
- The light primary action is `161 42% 18%` rather than a mid mint, so the
  focus ring can clear 3:1 on the button surface while still clearing 3:1 on
  white.

Mint is never the only carrier of meaning: every selected, safe, or cautionary
state also carries text, an icon, or a shape.

### Keyboard focus

`--ring` is the single focus token in both themes, applied through
`focus-visible:ring-2 focus-visible:ring-ring` (or the `.focus-ring` outline
helper). A focus ring is never removed without a visible replacement, and the
`.dark` and light values above are the measured pairs that keep it visible.

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
the OS shadow; the WebView is opaque and `border-radius` on it does not round
the OS window. The Quick Panel keeps its transparent, undecorated backing and
clips its own 20 px radius, which is what the shadow and hit testing follow.
Do not simulate traffic lights in CSS and do not add private window hacks to
chase a mockup radius.

## Motion specification

Motion communicates state change; it never delays input, replays a list, or
animates a number continuously.

### Material and emphasis

- New installations open in the light theme. A saved theme choice is retained.
- The sidebar and Quick Panel header/footer may blur the content behind them.
  The Overview cleanup focal surface is solid white with a single mint edge.
  File lists, category rows, dialogs, warnings, and selected cleanup targets
  also use solid surfaces.
- Keep gradients, decorative rings, inflated type, and repeated floating cards
  out of operational screens. Reduced transparency substitutes an opaque
  chrome surface, and reduced motion removes decorative movement.
- The cleanup workflow has one visible phase at a time: reviewed selection,
  execution, then inventory refresh. While refreshing after a completed clean,
  hide the old selection toolbar and category list. The result dialog opens
  after the new inventory is measured.

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
- **Badge**: status carries text meaning beyond colour (`success`, `warning`,
  `outline`), at `text-caption` or above.
- **ProgressBar**: 4–8 px tall, no looping shimmer, no indeterminate gradient.
- **ByteValue**: monospace tabular numerals for every byte metric.
- **SelectionToolbar**: reserved space in the content column; count, measured
  bytes, and risk summary, with one primary action.
- **SegmentedTabs**: `role="tablist"` with one `tabindex="0"`, `aria-selected`
  on the active tab, and `aria-controls` pointing at the panel; the selected
  tab uses the mint `--accent` surface rather than a dark pill.
- **Switch / Checkbox**: switches are for persistent on/off state only,
  checkboxes for selection. The enabled switch track and the selected checkbox
  use `--success` with a white check; never a plain white track.
- **EmptyState**: separate states for empty search results, no inventory,
  missing platform capability, and failed loading.
- **InlineNotice**: `role="status"` for information, `role="alert"` for errors,
  with the concrete reason and, when the action can be retried, one retry.

### Brand identity

- `BrandIcon` resolves an identity through one typed registry: a reviewed local
  asset when the copyright holder's licence clearly permits redistribution, and
  otherwise a neutral two-letter monogram beside the factual product name.
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

Compact heading, one cleanup summary with a `Review` action, CPU / memory
pressure / battery tiles showing their real values with per-domain freshness, a
compact list of active tools and services, and a compact Keep Awake control. At
960 × 660 the next action and the core metrics are visible without scrolling;
secondary detail scrolls. Each tile opens its exact detail page.

### Quick Panel

- At 320–360 px, use compact full-width reading rows. A label, current value,
  and one supporting fact should fit in roughly 48 px; details stay in the
  main window. The header and footer stay fixed while the body scrolls.
- Show each AI provider once. Merge observed sessions into its identity row,
  use a readable name and an explicit loading/stale/unavailable state, and
  express reset times with units rather than a bare minute counter.
- Cleanup and Keep Awake keep their actions within the same flat row hierarchy.
  The storage safety and freshness rules do not change with the presentation.

### Storage

- Top summary separates disk capacity, observed store size, the known cleanup
  estimate, and the selected amount. Unknown prune size can still be reviewed
  and cleaned.
- `Cleanup`: category rows with risk tiers, an actionable selection footer, and
  the existing backend eligibility, consent, and one-shot plan rules.
- `Developer Artifacts`: workspace/project list with generated-directory
  amounts, workspace authorization, and its own inventory freshness. No
  automatic whole-home scan and no unreviewed project deletion.
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

### Quick panel

- Designed natively at 360 × 520, tested at 320 px stress width and constrained
  heights. Fixed header and footer with at most one internal scrolling region.
- Default order: cleanup estimate and next action with `Review →`, CPU,
  memory pressure plus used amount, battery, disk capacity/free space, active AI
  and services with small identities and a count, Keep Awake state/duration.
  Saved order and visibility stay authoritative, and metric sections pair up
  two-per-row while full-width sections keep their own row.
- `Review` opens the relevant main-window review; it never deletes. The
  existing backend-owned Safe-only quick action stays available where eligible
  and is not broadened to models, volumes, or reviewed cache operations.
- Every summary has a real main-application destination. Tray anchoring,
  display clamping, native shadow, Escape/⌘W/focus dismissal, and
  hide-not-quit behaviour are preserved. A hidden panel polls nothing.

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
