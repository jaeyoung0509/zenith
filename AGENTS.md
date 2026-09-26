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

## Issue, PR, and version workflow

- Read this root `AGENTS.md` and `DESIGN.md` before implementation. Preserve and
  update these tracked files when an approved workflow or design changes; do not
  replace them with session notes or create a competing lowercase `agents.md`.
- Start from the current `develop` on an issue-scoped `feature/<issue>-<slug>`
  branch. Implement, verify, push, then open a PR targeting `develop`. Never
  commit directly to `main` or `develop`, and merge only after explicit user
  approval. Keep related requests in one PR when the user asks for one review unit.
- Every PR that changes shipped behavior or assets (including UI, logos, icons,
  and packaging) includes one patch bump by default. Documentation-only changes
  may retain the version; record that decision in the PR. An explicit user
  version instruction takes precedence. Do not silently omit the version step.
- Read the starting version with `just version`. Run `just bump-patch` once per
  PR, after its scope is settled and before final build verification. Use
  `just bump-minor`, `just bump-major`, or `just set-version <version>` only when
  that version change is requested. Never edit individual version fields by hand.
- Include all synchronized outputs in the same PR: `package.json`, root
  `Cargo.toml` `[workspace.package]`, the three workspace entries in `Cargo.lock`,
  and `src-tauri/tauri.conf.json`. Member manifests continue to inherit the
  workspace version. Run `just check-version` before committing and handing off.
- Follow-up commits on the same PR do not each get another bump. Compare against
  the latest target branch before handoff; if another merged PR consumed the
  proposed version, synchronize with that target and choose its next patch using
  the same recipes. Preserve unrelated work and resolve version conflicts together.
- Rebuild after the final version or asset change. Run the checks in Stack and
  commands above and inspect the `.app` bundle's version and packaged icon.
  A build is not an installation: state whether the running/installed app was
  actually replaced. Do not publish a release or tag as part of a version bump.
- The PR description and final handoff must identify linked issues, the version
  transition (or documented no-bump reason), checks run and their results, visual
  evidence for UI/asset changes, and any unverified platform behavior. Report CI
  status separately from local checks; do not call a pending check successful.
- Brand changes start at `src-tauri/icons/zenith-mark.svg`. Run
  `pnpm icons:generate` and `pnpm icons:check`, inspect app/compact/template
  variants at real display sizes, and update the branding contract in
  `DESIGN.md`. Do not redesign protected native glass as part of a logo change.

## Crate boundary

- Zenith is a Cargo workspace. `crates/zenith-core` owns product semantics,
  `crates/zenith-platform` owns the platform layer behind narrow ports
  (environment probing, path resolution, process control, bounded child
  execution, system actions, atomic file replacement, Trash), and the
  `src-tauri` package (`zenith-desktop`, library `zenith_lib`, binary `Zenith`)
  owns the Tauri adapter. The version, edition, and MSRV are stated once in the
  root `Cargo.toml` `[workspace.package]` table and all members inherit them;
  `just check-version` and `just bump-patch` maintain that one copy.
- Native code that belongs to another bounded context stays with its owner:
  keychain credentials in `ai_providers::credentials`, handle-level deletion
  safety in `safety`, allocated-size measurement in `scanner::size`, and
  process/machine introspection in `metrics`, `power`, `dev_ports`,
  `process_owner`, and `diagnostics`. Moving one of those into
  `zenith-platform` is its own change, not a boundary cleanup.
- `zenith-core` must not depend on `tauri`, `tauri-build`, `tauri-plugin-*`,
  `windows-sys`, `security-framework`, or `rfd`, directly or transitively, and
  `zenith-platform` must never reach `tauri`. Neither crate may depend on
  `zenith-desktop`: dependencies point into the domain, never back out of it.
  `scripts/check_core_boundaries.cjs` enforces these boundaries from
  `cargo metadata`; run `just check-architecture` after adding or moving a
  dependency.
- Ask of every Rust file: would this still make sense if Zenith had a CLI
  instead of a Tauri window? Domain semantics say yes and belong in
  `zenith-core`; native OS probing and OS API calls say yes and belong in
  `zenith-platform`. A file that owns WebView IPC, tray or window lifecycle,
  capability grants, or desktop composition is the only one that stays in
  `src-tauri`.
- Domain authorization state is not a frontend contract. `DeletePlan` and
  `DeleteTarget` carry no `serde` or `specta` derives; an interface-facing
  shape is a separate projection under `zenith_core::application::dto`.
- Do not restate a domain type in the desktop crate. `src-tauri/src/models`
  re-exports the core types by name so command modules keep one import root,
  and adds only the DTOs the desktop adapter produces itself.

## Tauri architecture

- Keep Tauri commands thin. Put scanning, cleanup, metrics, power management,
  and provider integrations in dedicated Rust modules and expose typed results.
- Keep `src-tauri/src/commands/mod.rs` as composition only. Add handlers to the
  closest domain module (`ai.rs`, `cleanup.rs`, or `system.rs`); dedicated
  storage workflows remain in `storage_commands.rs` as thin adapters over
  `services::StorageService`. `DesktopState` belongs in `state.rs`; the
  composition root (`src-tauri/src/composition`) builds it, `blocking.rs` owns
  worker execution and panic reporting, and `events/` owns every Tauri
  transport adapter. A command decodes IPC, calls one application service, and
  maps the typed result.
- Application services own the invariants their workflows depend on: the
  operation gate, the execution budgets, plan/inventory lifetimes, and the
  Trash executor. A command adapts transport (a `Channel`, an opaque ID) and
  never acquires the gate, stores a plan, or edits backend state itself.
- Run blocking filesystem, process, and HTTP work through
  `tauri::async_runtime::spawn_blocking`. Use Tauri channels for operations that
  report progress over time.
- Every serialized `u64` or `Option<u64>` that crosses IPC must use the shared
  `ipc_numeric` serde adapter (`zenith_core::ipc_numeric`, re-exported as
  `crate::ipc_numeric` in the desktop crate) plus a matching explicit Specta
  type annotation. Add a real model serialization regression test; never
  justify an unguarded integer with an assumed workstation-size bound.
- Lifecycle-owned registries such as cancellation handles must have a TTL and
  hard entry cap, recover poisoned locks where their state is disposable, and
  remove entries on success, cancellation, and error paths.
- Keep all `invoke` calls in `src/lib/utils/tauri.ts`. Dedicated storage-management workflows (Large Files Inspector and App Uninstaller) are the sanctioned exception: their native/browser-preview split lives in `src/lib/api/storage.ts` with selection via the shared `isTauri()` from `src/lib/api/index.ts`. Every other browser-previewed feature must have a deterministic mock guarded by `isTauri()`. See `docs/ARCHITECTURE.md` for the exact extension path.
- Register every application command in `src-tauri/build.rs` and grant it only to the windows that need it through `src-tauri/capabilities`. General destructive adapters belong to the main-window capability. The Quick Panel may expose backend-owned one-click cleanup only through a narrow Safe-only command that derives targets from the current trusted scan.
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
- Declare a platform capability `available` only when that platform has an
  implementing adapter and a test on that platform exercises it. A feature
  without an adapter must report `unavailable` with a reason instead of a
  successful empty result, and a value that cannot be derived on a platform
  must never be synthesized from an adjacent one. Do not leave an unreferenced
  capability snapshot in the tree as a record of intent; either route
  `current()` through it or delete it. Signatures whose paths resolve under a
  platform-specific root must declare `platforms`.

## Platform environment and verification

- Platform facts have one source: `zenith_platform::PlatformEnvironment` (built in
  `crates/zenith-platform`). It is built
  once at the composition root (`src-tauri/src/composition`), handed to the
  application services through `DesktopState`, and passed to scanning, cleanup,
  metrics, container, and storage code as an argument.
  Modules must not read `std::env` for paths, drives, or tool locations, and
  must not construct `PlatformEnvironment::native()` themselves.
- Windows path semantics live in `zenith_platform::path_algebra` as pure functions
  parameterized by `PathFlavor`. Add coverage there, not in a `#[cfg(windows)]`
  branch: a rule that only compiles on one platform is a rule only one runner
  checks. Ambiguous input (an 8.3 alias, a trailing dot or space, an
  alternate-data-stream colon) fails closed.
- `get_platform_context` is the interface's source for platform vocabulary
  (reveal label, recoverable-delete name, quick-panel surface, shortcut
  accelerator, log directory). Do not hardcode another platform's noun.
- Capability snapshots are exported as `src/lib/bindings/platform-capabilities.golden.json`
  by the ignored `tests::export_platform_capability_golden` test; frontend
  tests consume that file instead of a hand-written literal.
- `Zenith --doctor` prints a de-identified environment fingerprint and a
  self-check table sharing its assertions with CI, and exits 1 when a check
  fails. Committed fixtures in `src-tauri/tests/fixtures/environments` are
  picked up automatically by `tests/environment_fixture_tests.rs`; a pasted
  report becomes a permanent case for the cost of a commit.
- A test must not be gated on an environment variable, must not return early in
  place of asserting, and must not accept every variant of the value it is
  handed. `src-tauri/tests/test_hygiene.rs` fails new instances and keeps
  documented exceptions in `tests/hygiene_allowlist.txt`.

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
- Name domain states and user-facing actions once at their owning boundary.
  Avoid message-substring branching and repeated magic strings or numeric
  thresholds when they express the same rule. Use typed reasons for decisions;
  keep copy in a presentation helper where several views share it. Extract a
  shared helper only after confirming the callers have the same semantics, and
  keep deliberately different limits or platform contracts separate.
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
- Each platform Rust job runs `just check-architecture` alongside the format,
  lint, test, and check recipes. The crate boundary is a build invariant, not a
  review convention; do not move it behind a scheduled or advisory job.
- Packaging jobs use `.github/tauri.package-ci.json` to disable Tauri's frontend
  rebuild. Pass the file path to `--config`; do not inline JSON in workflow
  commands because PowerShell command forwarding strips nested quotes. The
  machine-wide NSIS variant is built from `.github/tauri.nsis-permachine.json`,
  which also disables the frontend rebuild.
- Release jobs fan out from one verified frontend artifact and fan in to one
  GitHub Release publisher. Only that publisher receives `contents: write`.
  Keep public artifact names stable and compute WinGet hashes from the final
  installer bytes. Generate and combine checksum manifests through
  `scripts/release_checksums.cjs` so every published file uses portable LF
  endings, then verify the combined manifest against the downloaded artifacts
  before publication. Never claim an unsigned build is signed; after SignPath
  approval, follow `CODE_SIGNING_POLICY.md` and verify Authenticode before
  checksum generation or publication.
- Documentation may not claim platform parity beyond what CI executes. A manual
  validation matrix must record each run's application version, OS build, and
  date, or be removed; an unexecuted matrix is a plan, not evidence.

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
- A catalog root may select (`paths` with `*` or `{a,b}`); nothing else may.
  Selector syntax in an exclusion or a prefix is a load error, because a pattern
  that matches no name protects nothing. Expansion is bounded, sorted, and never
  descends through a link, and the planner re-derives a path's authorizing roots
  from the pattern instead of trusting the scan.
- An observation-only entry (`strategy = "manual"`) may name an OS-owned root,
  because it reports bytes and never removes them; a deletable entry may not.
- Discovery never implies deletion permission. A discovered unit is reported
  with its observed bytes and an eligibility state even when it is recent,
  policy-gated, advisory, or blocked; never drop it to keep a total tidy. Only
  `AutoCleanable` is pre-selected, and `selected_bytes <= cleanable_bytes <=
  observed_bytes` holds for every total the interface shows.
- A cleanup unit is the object a plan authorizes: the configured path, an
  enumerated child, or a selected subtree. A unit's `cleanable_bytes` is what
  removing it reclaims: either the whole unit or — for a
  `delete_stale_contents` unit — exactly the entries whose own age was
  evaluated and will be re-evaluated at execution. Never report an estimate
  whose entries another rule would then refuse. Evaluate a whole-tree policy
  through `AgeObservation::evaluate` and a per-entry one through
  `StaleEntryPolicy::allows`, so the scan and the guard cannot drift.
- Structured-state classification, process guards, and the running-owner fact
  are re-derived at execution: a database, a lock, a credential, a
  configuration file, a bundle, or an executable inside a pruned tree stays,
  and a cache whose application is running is `reviewable` rather than
  automatic.
- Generic cleanup never removes structured state: databases, their WAL/SHM
  companions, locks, credentials, configuration, bundles, and executables are
  refused by the planner and skipped by the execution guard. A provider that
  owns a disposable store needs its own adapter.

## Product and design

- Treat `DESIGN.md` as the visual contract. Reuse existing tokens and shared
  components before adding new one-off styles.
- Keep copy concise and operational. For partial provider integrations, state
  precisely whether data is live, local, or manual instead of implying a quota
  is available.
- Keep unrelated user changes intact and avoid destructive Git commands.
