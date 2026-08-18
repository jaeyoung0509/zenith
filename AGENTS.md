# Zenith Engineering Guide

These instructions apply to the entire repository. Preserve the product and
safety conventions below when changing Zenith.

## Stack and commands

- Desktop shell: Tauri 2 on macOS, with Rust commands in `src-tauri/src`.
- UI: Svelte 5, TypeScript, Vite, and Tailwind CSS in `src`.
- Package manager: pnpm. Task runner: `just`.
- Use `pnpm install` for dependencies, `just dev` for the desktop dev loop, and
  `just dev-web` for browser-only UI work with mocked IPC.
- `just build-fast` creates a debug `.app` bundle and `just run-fast` opens that
  bundle. Do not switch it back to launching the bare Mach-O binary; macOS only
  applies the configured Dock/Finder icon reliably to the application bundle.
- Before handing off a change, run `cargo check`, `cargo test`, `pnpm check`,
  `pnpm test -- --run`, and `pnpm build`. Use `just build-fast` to verify that
  the standalone debug binary embeds the current frontend.

## Tauri architecture

- Keep Tauri commands thin. Put scanning, cleanup, metrics, power management,
  and provider integrations in dedicated Rust modules and expose typed results.
- Run blocking filesystem, process, and HTTP work through
  `tauri::async_runtime::spawn_blocking`. Use Tauri channels for operations that
  report progress over time.
- Keep all `invoke` calls in `src/lib/utils/tauri.ts`. Every browser-previewed
  feature must have a deterministic mock guarded by `isTauri()`.
- Never read or expose OAuth credential files directly. Prefer an official CLI
  or API flow. Keep provider secrets in Rust, return only derived usage data,
  and use the OS keychain if persistence is introduced. Never log tokens.
- The Cargo default feature must include `custom-protocol`; otherwise a binary
  built outside `tauri dev` opens a blank webview because the frontend is not
  embedded.
- Create the tray icon once in Rust. The tray menu must include Open Zenith,
  Toggle Quick Panel, and Quit Zenith.
- Position the quick panel from the tray click coordinates, clamp it to the
  active display, and align its right edge beneath the menu-bar icon.
- Window labels are `main` and `quick`. Closing the quick panel hides it; it
  must not terminate the background app. Every frameless window needs an
  obvious keyboard-accessible close control.
- Persist user preferences as validated JSON in Tauri's app configuration
  directory. Missing fields must deserialize to safe defaults so upgrades do
  not discard existing settings. Reload preferences when the persistent quick
  window is activated because each webview owns a separate frontend store.
- Hidden quick panels must not poll metrics or invoke provider CLIs. Disk data
  refreshes once per activation, memory may poll only while visible, and AI
  provider snapshots use a bounded backend cache with manual refresh support.
- Never expose arbitrary PID kill commands. Memory actions must resolve a fresh
  process snapshot from an allowlisted user-app group (including executables in
  installed `.app` bundles), protect system/terminal/Zenith processes, and
  offer graceful termination before force termination.
- Native app selection for Keep Awake starts in `/Applications`, reads
  `CFBundleExecutable`, and returns only the display name, executable name, and
  bundle path. Cancellation is a normal empty result, not an error.

## Svelte conventions

- Use Svelte 5 runes (`$state`, `$derived`) and typed component props. Avoid a
  local identifier named `state`, which is easy to confuse with the `$state`
  rune.
- Keep route views focused on composition. Shared controls belong in
  `src/lib/components`, stateful domain logic in `src/lib/stores`, and IPC types
  in `src/lib/models/types.ts`.
- An `onMount` callback must return cleanup synchronously. Start async work from
  inside it rather than making the callback itself async.
- Icon-only buttons require an accessible label and tooltip. Preserve visible
  loading, empty, error, disabled, hover, and focus states.
- Do not duplicate backend business rules in the UI. Browser mocks should match
  the real command response shape, not become a second implementation.

## Cleanup safety invariants

- Cleanup targets originate from registered TOML signatures. The planner must
  reject paths outside a signature's resolved scope.
- Never scan or delete all of `/tmp`. Temporary cleanup is limited to direct
  children with known tool prefixes and a minimum inactivity age. Determine
  inactivity from the newest timestamp in the candidate tree.
- Do not follow symlinks. Apply blacklist, signature-scope, and TOCTOU checks
  before deletion. Only `Safe` items may be auto-selected; `Rebuild` and
  `Manual` items require explicit user intent.
- Add a regression test for every safety boundary change. Tests must use
  temporary fixtures and must never clean real user directories.
- Do not include nonexistent or zero-byte signature paths in scan results.
  Order cleanup candidates by reclaimable bytes unless the user chooses another
  explicit sort. `Rebuild` means deletable but re-downloadable/rebuildable; it
  remains opt-in to avoid unexpected network or build costs.

## Product and design

- Treat `DESIGN.md` as the visual contract. Reuse existing tokens and shared
  components before adding new one-off styles.
- Keep copy concise and operational. For partial provider integrations, state
  precisely whether data is live, local, or manual instead of implying a quota
  is available.
- Keep unrelated user changes intact and avoid destructive Git commands.
