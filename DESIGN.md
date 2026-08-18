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

Use the system sans-serif stack for labels and prose. Use monospace numerals for
bytes, percentages, token counts, prices, process IDs, and reset times. Body text
is generally 12–14 px; section titles 14–16 px; headline metrics 28–32 px.

## Layout

- Main window baseline: 960 × 660 px with a 224 px left sidebar.
- Quick panel baseline: 360 × 520 px. It must stay useful above other windows,
  with a close button at the upper right and no hidden essential controls.
- Main content uses a 24–32 px outer inset, 16–24 px section gaps, and 12–16 px
  internal card padding. Preserve the compact density visible in the current
  Storage and AI Usage screens.
- The macOS traffic-light area and titlebar drag region must remain unobstructed.
  Interactive elements inside a drag region require the `no-drag` class.

## Components and interaction

- Reuse `Card`, `Button`, `Badge`, and `ProgressBar`. A page should have one
  visually dominant primary action; supporting actions stay secondary or ghost.
- Rows use icon, title, secondary metadata, metric, and disclosure/action in that
  order. Align metrics vertically and keep labels short enough to scan.
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

### Quick panel

- Show the high-frequency snapshot only: storage, safe reclaim amount, category
  totals, memory, Scan Again, and Open Zenith.
- The close button hides the panel while the menu-bar process continues. Quit is
  intentionally available from the tray menu, not conflated with close.
- Open directly below the clicked menu-bar icon, right-aligned to it and clamped
  inside the active display. Do not reuse a stale centered window position.

### Disk management

- Separate cleanup candidates from physical/logical volume health. The Disks
  view shows mounted volumes, used/free capacity, mount points, and a handoff to
  macOS Disk Utility; destructive volume operations remain outside Zenith.
- Default cleanup ordering is largest reclaimable size first. Hide nonexistent
  and zero-byte signature locations. Explain that Rebuild items are deletable
  but intentionally opt-in because the next tool run may need network downloads
  or recompilation.

## Visual QA checklist

- Compare both the main window and quick panel at their baseline sizes.
- Check long labels, zero values, offline providers, loading, failure, and large
  numbers. Verify text contrast and that nothing clips behind the titlebar.
- Confirm the app, Dock, title, and tray icons use the intended variants and do
  not appear twice.
- Test keyboard focus and screen-reader names for every clickable control.
