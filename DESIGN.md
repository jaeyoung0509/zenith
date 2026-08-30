# Zenith Design System

Zenith is a compact, native-feeling macOS utility for developers. The current
dark dashboard is the visual baseline: quiet surfaces, dense information, clear
safety signals, and one obvious action per section.

## Product character

- Native macOS utility, not a marketing dashboard.
- Calm, technical, and trustworthy. Avoid decorative gradients, glass effects,
  oversized headings, excessive pills, and motion without feedback value.
- The white circular `Z` mark is the product identity. Use the template-style
  monochrome variant for the menu bar and the full app icon for Finder, Dock,
  title areas, and application menus.

## Foundation

The canonical color tokens live in `src/app.css`. In dark mode:

- Background: near-black neutral `240 10% 7%`.
- Cards: `240 10% 10%`; elevated or interactive surfaces use the secondary and
  accent tokens rather than arbitrary gray values.
- Borders: subtle `240 4% 18%`, normally one pixel.
- Primary text: near white; secondary text uses `muted-foreground`.
- Corners use `--radius: 0.65rem`. Small controls may use a slightly smaller
  radius; reserve fully rounded shapes for status badges and progress details.
- Emerald means safe, protected, or connected. Amber means rebuild or warning.
  Red means destructive or failed. Violet identifies AI usage without replacing
  the semantic status colors.
- Enabled switches use emerald track fill with a white thumb. Never use a plain
  white track for the enabled state because it is indistinguishable from an
  inactive control on light surfaces.

Use the system sans-serif stack for labels and prose. Use monospace numerals for
bytes, percentages, token counts, prices, process IDs, and reset times. Body text
is generally 12–14 px; section titles 14–16 px; headline metrics 28–32 px.
Compact supporting copy uses the named Tailwind steps `text-micro` (9 px),
`text-caption` (10 px), and `text-meta` (11 px); do not reintroduce arbitrary
pixel utilities for those sizes.

## Layout

- Main window baseline: 960 × 660 px with a 224 px expanded sidebar. The sidebar
  can collapse to an icon rail; the toggle remains visible, labelled, and
  keyboard accessible in both states. Persist that preference with validated
  settings so a relaunch does not unexpectedly change the user's layout.
- Quick panel baseline: 360 × 520 px. It must stay useful above other windows,
  with a close button at the upper right and no hidden essential controls.
- Main content uses a 24–32 px outer inset, 16–24 px section gaps, and 12–16 px
  internal card padding. Preserve the compact density visible in the current
  Storage and AI Usage screens.
- Keep the main content fluid when the sidebar changes width. Navigation labels
  may hide in the collapsed rail, but icons need a tooltip and an accessible
  name; do not remove the active, focus, or safety states.
- The macOS traffic-light area and titlebar drag region must remain unobstructed.
  Interactive elements inside a drag region require the `no-drag` class.

## Components and interaction

- Reuse `Card`, `Button`, `Badge`, and `ProgressBar`. A page should have one
  visually dominant primary action; supporting actions stay secondary or ghost.
- Rows use icon, title, secondary metadata, metric, and disclosure/action in that
  order. Align metrics vertically and keep labels short enough to scan.
- Comparable values (bytes, percentages, counts, and prices) use a stable
  right-aligned column with monospace numerals and `white-space: nowrap`.
  Action buttons also stay on one line; descriptive copy may wrap or truncate
  inside a `min-width: 0` content column instead of pushing metrics around.
- Status indicators must be data-backed and self-explanatory. A Storage indicator
  means reclaimable data is available; expose the amount in the expanded rail
  as a quiet inline dot-plus-monospace value and an accessible label/tooltip in
  the collapsed rail. Do not use a high-contrast pill or add decorative dots to
  tabs that have no pending state.
- Sidebar collapse is a low-emphasis chevron control: it gains a soft surface
  and border only on hover/focus, while the icon remains directional and the
  accessible label describes the resulting action (Expand or Collapse).
- Every action needs hover, keyboard focus, disabled, loading, success, and error
  behavior where applicable. Icon-only actions require labels and tooltips.
- Loading should preserve the surrounding layout. Empty states explain what is
  missing and give the next useful action. Errors appear near the failed action
  and must not erase the last successful data.
- Prefer brief 120–180 ms color/opacity transitions. Avoid layout-shifting or
  looping animation except for active progress indicators.

## Feature-specific patterns

### Storage

- Put disk usage, selected reclaimable space, and the cleanup action in the top
  summary card. Category rows retain safety tier, selected amount, and total.
- Safe items may begin selected; Rebuild and Manual items never do.

### AI Usage

- Each provider card clearly labels its data source as Live, Local, or Manual.
- Codex may show OAuth account windows and local token summaries. OpenCode shows
  local activity only. OpenRouter shows live key usage after an explicit OAuth
  connection. Claude Code and Antigravity remain honest manual handoffs until
  their vendors expose suitable account-usage APIs.
- Never show account email addresses or secret-bearing identifiers. Connection
  buttons describe the provider and open only user-initiated OAuth flows.

### AI Control Center

- Use four compact sections: Overview, Usage & Budgets, Resource Autopilot, and
  Safety Posture.
- Provider rows always show provenance scope and freshness badges (`Fresh`, `Stale`,
  `Partial`, `Unavailable`); local estimates and manual values must never look like
  provider-enforced quotas.
- Local alert thresholds must be labelled as "Zenith local budget alert" and must
  not imply provider billing or quota enforcement. When aggregating across different
  provenance kinds (e.g. authoritative + local estimate + manual), copy must
  explicitly indicate "mixed sources".
- Recommendations are presented as advisory cards with clear action labels.
  Preview modals clearly inform the user that one-shot previews expire and that
  no mutation has occurred.
- Safety findings are displayed with distinct severity badges (`Critical`, `Warning`,
  `Info`), concise remediation steps, relative file locations, and dismiss controls.
- Automation switches state that they are off by default and emphasize that policies
  never terminate processes, release ports, or delete files automatically.
- The Quick Panel shows only the cached compact summary and offers no refresh or
  mutation control for this feature.

### Quick panel

- Show only user-selected high-frequency sections: storage, safe reclaim amount,
  compact AI usage, category totals, and memory. Preserve the saved section and
  AI-provider priority order; Scan Again and Open Zenith remain fixed actions.
- The close button, Escape, Cmd+W, and focus loss hide the panel while the
  menu-bar process continues. Quit is intentionally available from the tray
  menu, not conflated with close.
- Open directly below the clicked menu-bar icon, right-aligned to it and clamped
  inside the active display. Do not reuse a stale centered window position.
- Hidden panels perform no recurring metrics polling or provider collection.
  Refresh disk metrics once when opening, poll memory only while visible, and
  keep AI usage bounded by a short cache with an explicit refresh action.

### Disk management

- Separate cleanup candidates from physical/logical volume health. The Disks
  view shows mounted volumes, used/free capacity, mount points, and a handoff to
  macOS Disk Utility; destructive volume operations remain outside Zenith.
- Default cleanup ordering is largest reclaimable size first. Hide nonexistent
  and zero-byte signature locations. Explain that Rebuild items are deletable
  but intentionally opt-in because the next tool run may need network downloads
  or recompilation.

### Memory inspector

- Termination is available only for recognized user applications, including
  executables inside installed `.app` bundles. Keep the
  action visually secondary until row hover and always show a confirmation with
  process count, estimated memory, and unsaved-work warning.
- Offer normal Quit before the red Force Quit action. Explain that displayed
  memory is an estimate and macOS may retain released pages as reusable cache.
- Keep this view focused on memory-ranked application groups.

### Development Servers

- Use a dedicated sidebar tab rather than mixing listeners into the Memory
  inspector. Rows lead with port/protocol, then sanitized server and project
  context, bind exposure, process age, and the secondary Release action.
- Recognized disposable testing infrastructure such as agent-browser and Chrome
  for Testing follows the same row and confirmation treatment as development
  servers; do not imply that ordinary browser sessions are releasable.
- Use neutral styling for loopback, an informational treatment for a specific
  network interface, and the semantic warning token for all-interface binds.
  Protected or unrecognized listeners remain visible with the backend-provided
  reason but have no destructive action.
- Graceful release is the only action in the first dialog. Show the destructive
  Force Release dialog only after the backend confirms the same listener
  ignored SIGTERM. Dialog focus enters on open and returns to the originating
  row action on cancel or completion.

### Projects

- Keep AI Activity Level 1 focused with three compact sub-tabs directly below
  the header: `Usage` (default), `Projects`, and `Tool Adapters`. Each tab
  should show only its own domain and use an obvious active indicator plus a
  visible keyboard focus ring; the tab state must not rely on color alone.
- Scope loading and refresh feedback to the selected sub-tab. Usage displays
  provider cards, Projects displays project/session summaries, and Tool
  Adapters displays the supported adapter matrix. Preserve successful stale
  data beside an inline error and load agent integrations only when the
  adapter tab is first opened.
- Group rows by canonical repository/worktree identity and use the compact
  parent/name hint plus branch to distinguish same-name projects without showing
  an absolute path. Worktrees receive a text badge.
- Every session shows a non-color evidence label. Process-only observations say
  `Process observed · detailed status unavailable`; they never claim Finished,
  Waiting, or Stalled.
- Keep Unassigned sessions visible in their own section with an explanation that
  project correlation could not be proven. Never guess from basename, branch,
  port, or PID.
- Preserve the last successful snapshot during refresh failures. First load uses
  stable skeleton cards; empty state explains supported local observation and
  the privacy boundary. Adapter health stays available in a secondary disclosure.

### Keep Awake

- “Add Rule” leads with a native Applications picker and automatically fills the
  app name and executable. Manual fields remain below a divider for CLI tools.
- Show the selected bundle path so the user can verify the application before
  saving the rule.

## Visual QA checklist

- Compare both the main window and quick panel at their baseline sizes.
- Check expanded and collapsed sidebars, long labels, zero values, offline
  providers, loading, failure, and large numbers. Verify that byte metrics and
  action labels do not wrap inconsistently, that long descriptive copy truncates
  gracefully, and that nothing clips behind the titlebar.
- Confirm the app, Dock, title, and tray icons use the intended variants and do
  not appear twice.
- Test keyboard focus, tooltips, and screen-reader names for every clickable
  control, including the sidebar toggle and Storage status affordance.
