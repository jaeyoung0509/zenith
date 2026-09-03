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
- Keep `src-tauri/src/commands/mod.rs` as composition only. Add handlers to the
  closest domain module (`ai.rs`, `cleanup.rs`, or `system.rs`); dedicated
  storage workflows remain in `storage_commands.rs`. Shared command state and
  helpers belong in `state.rs` and `support.rs`, not in the composition root.
- Run blocking filesystem, process, and HTTP work through
  `tauri::async_runtime::spawn_blocking`. Use Tauri channels for operations that
  report progress over time.
- Every serialized `u64` or `Option<u64>` that crosses IPC must use the shared
  `ipc_numeric` serde adapter plus a matching explicit Specta type annotation.
  Add a real model serialization regression test; never justify an unguarded
  integer with an assumed workstation-size bound.
- Lifecycle-owned registries such as cancellation handles must have a TTL and
  hard entry cap, recover poisoned locks where their state is disposable, and
  remove entries on success, cancellation, and error paths.
- Keep all `invoke` calls in `src/lib/utils/tauri.ts`. Dedicated storage-management workflows (Large Files Inspector and App Uninstaller) are the sanctioned exception: their native/browser-preview split lives in `src/lib/api/storage.ts` with selection via the shared `isTauri()` from `src/lib/api/index.ts`. Every other browser-previewed feature must have a deterministic mock guarded by `isTauri()`. See `docs/ARCHITECTURE.md` for the exact extension path.
- Register every application command in `src-tauri/build.rs` and grant it only to the windows that need it through `src-tauri/capabilities`. Destructive adapters belong to the main-window capability, not the quick panel.
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
- Windows native pickers must use static, non-interpolated PowerShell/COM
  scripts, explicitly decode UTF-8 output, and treat user cancellation as an
  empty result rather than an error.

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
- Keep browser-preview fixtures in domain files under `src/lib/api/mocks` and
  preserve exact top-level key parity with the native API via contract tests.
- Recurring store work must be owned by explicit subscribers. Use reference
  counting so one consumer cannot stop another, start no timer in constructors,
  and cover start/stop/idempotency with fake-timer tests.

## CI boundaries

- Keep frontend typecheck, Vitest, binding drift, and Vite build in the shared
  frontend job. macOS and Windows Rust jobs run in parallel; packaging jobs may
  consume the verified frontend artifact only after their platform Rust job
  succeeds. Do not duplicate the frontend test suite in each platform job.
- Packaging jobs use `.github/tauri.package-ci.json` to disable Tauri's frontend
  rebuild. Pass the file path to `--config`; do not inline JSON in workflow
  commands because PowerShell command forwarding strips nested quotes.
- Release jobs fan out from one verified frontend artifact and fan in to one
  GitHub Release publisher. Only that publisher receives `contents: write`.
  Keep public artifact names stable and compute WinGet hashes from the final
  installer bytes. Generate and combine checksum manifests through
  `scripts/release_checksums.cjs` so every published file uses portable LF
  endings, then verify the combined manifest against the downloaded artifacts
  before publication. Never claim an unsigned build is signed; after SignPath
  approval, follow `CODE_SIGNING_POLICY.md` and verify Authenticode before
  checksum generation or publication.

## Cleanup safety invariants

- Cleanup targets originate from registered TOML signatures. The planner must
  reject paths outside a signature's resolved scope.
- Delete plans remain private Rust values in the bounded plan store. The
  frontend submits only the current scan ID, selected item IDs, and then the
  opaque one-shot plan ID; it never supplies paths, strategies, or identities.
- `Manual` is never a generic filesystem strategy. Stateful resources such as
  local models and Docker volumes require a typed adapter and explicit UX.
- Never scan or delete all of `/tmp`. Temporary cleanup is limited to direct
  children with known tool prefixes and a minimum inactivity age. Determine
  inactivity from the newest timestamp in the candidate tree.
- Do not follow symlinks. Apply blacklist, signature-scope, and TOCTOU checks
  before deletion and at every recursive entry. Only `Safe` items may be
  auto-selected; `Rebuild` requires explicit preference or selection and
  `Manual` is never selected for generic cleanup.
- Add a regression test for every safety boundary change. Tests must use
  temporary fixtures and must never clean real user directories.
- Do not include nonexistent or zero-byte signature paths in scan results.
  Order cleanup candidates by reclaimable bytes unless the user chooses another
  explicit sort. `Rebuild` means deletable but re-downloadable/rebuildable; it
  remains opt-in to avoid unexpected network or build costs.
- Tool-owned shared caches must use a backend-owned fixed-argument provider or
  remain advisory. `external_command` is never a filesystem-delete fallback;
  rediscover and validate the provider path immediately before mutation.

## Product and design

- Treat `DESIGN.md` as the visual contract. Reuse existing tokens and shared
  components before adding new one-off styles.
- Keep copy concise and operational. For partial provider integrations, state
  precisely whether data is live, local, or manual instead of implying a quota
  is available.
- Keep unrelated user changes intact and avoid destructive Git commands.
