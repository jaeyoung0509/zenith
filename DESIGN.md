# Zenith Design System

Zenith is a compact, native-feeling macOS and desktop utility for developers.
The visual baseline features quiet surfaces, dense technical information, clear
safety signals, and one obvious action per section.

## Product character

- Native desktop developer utility, not a marketing dashboard.
- Calm, technical, and trustworthy. Avoid decorative gradients, glass effects,
  oversized hero metrics, excessive pills, and motion without feedback value.
- The white circular `Z` mark is the product identity. Use the template-style
  monochrome variant for the menu bar and the full app icon for Finder, Dock,
  title areas, and application menus.

## Foundation and semantic tokens

The canonical color and motion tokens live in `src/app.css` and `tailwind.config.js`.

### Color and surface hierarchy

- **Dark mode (neutral charcoal)**:
  - Window background: `hsl(240 10% 7%)`
  - Cards & Content surfaces: `hsl(240 10% 10%)`
  - Elevated surfaces & Secondary controls: `hsl(240 4% 16%)`
  - Borders: subtle `hsl(240 4% 18%)`, normally 1 px
  - Primary text: `hsl(0 0% 98%)`; secondary/supporting text: `hsl(240 5% 65%)` (`muted-foreground`)
- **Light mode (clean warm neutral)**:
  - Window background: `hsl(240 10% 98%)`
  - Cards & Content surfaces: `hsl(0 0% 100%)`
  - Elevated surfaces & Secondary controls: `hsl(240 5% 94%)`
  - Borders: `hsl(240 6% 88%)`
  - Primary text: `hsl(240 10% 10%)`; secondary text: `hsl(240 4% 46%)` (`muted-foreground`)

### Semantic safety and status palette

- **Emerald (`--success`)**: `hsl(158 64% 52%)` dark / `hsl(161 94% 30%)` light.
  Represents Safe cleanup candidates, protected items, active network connections, and healthy metrics.
- **Amber (`--warning`)**: `hsl(43 96% 56%)` dark / `hsl(32 95% 44%)` light.
  Represents Rebuild caches, cautionary states, elevated memory pressure, and stale scans.
- **Red (`--destructive`)**: `hsl(0 72% 51%)` dark / `hsl(0 84% 60%)` light.
  Represents destructive actions, force termination, process kills, and hard errors.
- **Violet (`--ai`)**: `hsl(265 85% 68%)` dark / `hsl(265 75% 55%)` light.
  Identifies AI Activity, models, and provider metadata without replacing semantic safety signals.
- Enabled switches use emerald track fill with a white thumb. Never use a plain
  white track for the enabled state because it is indistinguishable from an
  inactive control on light surfaces.

### Typography scale

- Use the system sans-serif stack for labels, navigation, and prose.
- Use tabular monospace numerals (`font-mono tabular-nums`) for bytes, percentages,
  token counts, prices, ports, process IDs, and countdown timers.
- Key headline metrics: 28–32 px (`text-2xl` / `text-3xl font-bold font-mono`)
- Page headings: 18–20 px (`text-lg font-semibold tracking-tight`)
- Section headings: 14–15 px (`text-sm font-semibold tracking-tight`)
- Body & interactive controls: 13–14 px (`text-xs` / `text-sm font-medium`)
- Supporting metadata: 11 px (`text-meta`)
- Captions & timestamps: 10 px (`text-caption`)
- Micro badges & status dots: 9 px (`text-micro`)

### Spacing and sizing scale

- 4 px spacing scale: 4 / 8 / 12 / 16 / 24 / 32 px.
- Standard interactive row height: 40–44 px for main window; 32–36 px for compact quick panel.
- Control click targets: 32–36 px height; icon-only button hit area minimum 28 × 28 px.
- Radii: 8 px (`rounded-lg`) for controls/buttons, 12 px (`rounded-xl`) for cards and grouped surfaces, 14–16 px (`rounded-2xl`) for modal overlays.

## Motion specification

Polish with a strict purpose: animations must communicate state change and provide feedback without causing layout shifts, CPU wakeups, or input latency.

| Interaction | Duration & Easing | Behavior |
| --- | --- | --- |
| Hover / focus / pressed | 90–120 ms (`--duration-instant`, `cubicOut`) | Subtle color/opacity feedback; avoid moving dense rows or shifting text |
| Selection / tab indicator | 120–160 ms (`--duration-fast`, `cubicOut`) | Smooth indicator transition, content becomes interactive immediately |
| Page content change | 140–180 ms (`--duration-normal`, `cubicOut`) | Subtle opacity transition with at most 2–4 px translation; no exit-before-enter delay |
| Dialog / detail panel | 160–220 ms (`--duration-overlay`, `cubicOut`) | Soft opacity + slight scale transform; focus trapped and restored |
| Refresh | Immediate pending state | Keep existing data stable, update changed values without animated counters or list replay |
| Sidebar collapse | At most 180 ms | Width and padding transition with label fade; main content remains fluid |

- Centralize easing and durations using CSS custom properties (`--duration-instant`, `--duration-fast`, `--duration-normal`, `--duration-overlay`, `--ease-out-cubic`).
- Honor `prefers-reduced-motion`: all spatial movement and transitions are instantly zeroed, while preserving visual feedback and text updates.

## Layout and information architecture

### Main window baseline

- Default size: 960 × 660 px; minimum size: 800 × 560 px.
- Expanded sidebar width: 224 px (`w-56`); collapsed sidebar width: 64 px (`w-16`).
- Main content outer inset: 24–32 px (`p-6` to `p-8`), 16–24 px section gaps.
- Native drag region: Top 28 px (`h-7`) unobstructed for macOS titlebar drag. Interactive items inside require the `no-drag` class.

### Navigation and page hierarchy

Organize the default sidebar into subtle visual groups when space permits (sidebar expanded):

1. **Storage**:
   - Primary destinations: Storage (`storage`), Containers (`docker`), Local Models (`models`).
   - Storage exposes discoverable secondary navigation: `Cleanup` | `Developer Artifacts` | `Large Files` | `Applications` | `Disks`.
2. **Runtime**:
   - Memory (`memory`), Dev Servers (`development_servers`), Keep Awake (`awake`).
3. **AI**:
   - AI Activity (`projects`): Consolidated hub with subtabs for `Usage`, `Projects`, `Tool Adapters`, and direct access to `AI Control Center`.
4. **Preferences**:
   - Settings (`settings`): Fixed at the bottom of the sidebar.

*Preservation rule*: User-customized sidebar tab visibility and ordering (`settings.dashboard_tabs`) is strictly respected; custom configurations are never reset or reordered.

### Standard page layout pattern

Each view follows:
**Page header (title, icon, contextual action) → compact summary (if useful) → filters / search / selection → primary content rows → detail drawer/panel on demand**.

## Components and interaction contract

- **Button**: Semantic variants (`primary`, `secondary`, `outline`, `ghost`, `destructive`), standardized sizes (`xs`, `sm`, `md`, `icon`), visible `focus-visible:ring-2 focus-visible:ring-ring`, disabled explanation.
- **Card**: Clean surface separation with `border border-border/70 bg-card/60`, rounded-xl, no decorative glow or artificial blur.
- **Badge**: Status indicators with text meaning beyond color (`variant="success"`, `variant="warning"`, `variant="outline"`).
- **ProgressBar**: Restrained height (4–8 px), smooth progress without continuous looping shimmer.
- **ByteValue**: Monospace tabular-numeral formatting for all byte metrics.
- **SelectionToolbar**: Reserved space inside the content area for bulk actions; count, estimated bytes, and risk summary.
- **EmptyState**: Distinct states for empty search results, no inventory, missing platform capability, and failed loading.

## Feature-specific patterns

### Storage
- Top summary card separates disk capacity from cleanup eligibility and selected bytes.
- Secondary navigation tabs allow switching between:
  - `Cleanup`: Category list, risk tiers, selection summary, review and clean.
  - `Developer Artifacts`: Reviewable project build directories (node_modules, target, venv).
  - `Large Files`: Size-sorted files with trash workflow.
  - `Applications`: Installed app bundles and associated caches.
  - `Disks`: Mounted volume capacity and macOS Disk Utility handoff.
- Mixed-risk selections replace ambiguous "Clean Safely" with "Review & Clean".

### Quick panel
- Default size: 360 × 520 px. Works smoothly at short heights and down to 320 px stress width.
- Glancable top summary: System Status / Ready to Clean with one-click Safe Clean action.
- Configurable user sections rendered in saved order (`settings.quick_panel_sections`).
- Immediate cached data rendering with zero hidden-window recurring timers or polling.
- Fixed header with Close and Open Zenith buttons; fixed footer with Rescan and version tag.

### Containers (Docker)
- Distinguish daemon running vs stopped vs not installed.
- Safe pruning of dangling images and build cache; protected volumes require explicit confirmation.

### Local models
- Provider context (Ollama, HuggingFace, LM Studio, Apple MLX), model sizes, locations, and manual deletion handoff.

### Memory inspector
- System memory pressure as the primary health signal; top memory-ranked user applications; graceful Quit before Force Quit.

### Development servers
- Port and protocol first, then project/tool and network exposure (loopback vs all-interfaces); graceful Release before Force Release.

### AI Activity & Projects
- Subtabs: `Usage`, `Projects`, `Tool Adapters`, and `AI Control`.
- Non-color evidence labels on all observed sessions; worktree badges; return from project details without losing state.

### AI Control Center
- Provenance-aware metrics; local budget alert thresholds clearly distinguished from provider-enforced quotas; advisory safety posture.

### Keep Awake
- Prominent active state and remaining time; separation of manual timer from application rules; native application picker.

### Settings
- Grouped sections: Appearance (System/Light/Dark), Navigation & Quick Panel, Cleanup, Providers, Notifications & Automation, Diagnostics.

## Accessibility and visual QA

- WCAG AA contrast: Normal text ≥ 4.5:1, large text and boundaries ≥ 3:1 in both light and dark themes.
- Full keyboard traversal with visible focus indicators (`focus-visible:ring-2 focus-visible:ring-ring`).
- Accessible names, tooltips, and labels on all icon-only buttons.
- No information conveyed by color alone.
