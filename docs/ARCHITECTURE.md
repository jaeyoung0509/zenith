# Architecture

Zenith is a cross-platform desktop application built with Tauri 2, Rust, Svelte 5,
and TypeScript, supporting macOS and Windows x64.
Rust owns system access, security boundaries, and destructive decisions. Svelte
renders typed state and submits user intent; it never constructs or coordinates
raw filesystem operations.

## Runtime shape

```text
macOS menu bar / Windows system tray
     |
     +-- quick WebView (hidden until requested)
     |      read metrics, scan, execute a backend-owned safe plan
     |
     +-- main WebView
            dashboard, settings, reviewed destructive adapters
                    |
                    | typed Tauri IPC (specta bindings, async spawn_blocking)
                    v
              Rust application core (AppState)
        +-----------+-----------+-------------+
        |           |           |             |
     scanner      safety      adapters      metrics/power
        |        planner +     Docker,       memory, disk,
   signatures    plan store    models, AI    Keep Awake
        |
        +-- dedicated storage management (StorageWorkflowState)
            large-file inventory, app inventory,
            one-shot Trash plans

        +-- developer artifact review (StorageWorkflowState)
            picker-owned workspace roots, marker discovery,
            bounded candidate measurement, one-shot Trash plans

        +-- development-port management (DevelopmentPortStore)
            listener discovery + classification,
            one-shot leases, exact-process signaling

        +-- AI control plane (AiControlRuntime)
            event-driven condvar wake loop, process activity,
            advisories, safety audit, git baselines
```

The windows have separate frontend runtimes and stores. Shared authority and
coordination therefore live in Rust `AppState`, backend-owned workflow states, or
Rust process state rather than in a browser singleton. Lifecycle-owned storage
state uses poison recovery for bounded inventories, plans, workspace
registrations, and cancellation handles. Other domain locks fail closed when
their state cannot be trusted.

## Platform capability contract

Platform-specific behavior is selected behind Rust service boundaries instead
of being spread through route components. `src-tauri/src/platform` owns the
runtime platform contract and `PlatformCapabilitiesProvider` exposes a typed,
read-only snapshot through `get_platform_capabilities`. The frontend loads the
snapshot independently in each WebView and uses it to hide or disable actions
that are unavailable on the current platform. A `read_only` capability may
continue to expose inspection metrics, but mutating controls require an
`available` capability.

Both macOS and Windows x64 implement the capability contract across all thirteen
core features. Platform-specific actions, including workspace selection, use
native adapters tailored to each OS; unsupported platforms report unavailable
capabilities instead of exposing nonfunctional controls.

Workspace selection uses `NSOpenPanel` on macOS and the Windows Shell COM folder
picker through a static PowerShell script on Windows. The Windows adapter never
interpolates user-controlled text into the script, requests UTF-8 output, and
maps picker cancellation to `None` just like the macOS adapter.

## Repository map

- `src-tauri/src/commands`: narrow generic IPC boundary. `mod.rs` only composes
  domain exports; `ai.rs`, `cleanup.rs`, and `system.rs` own handlers, while
  `state.rs` and `support.rs` own shared state and helpers.
- `src-tauri/src/platform`: platform capability contract and native provider
  composition seams.
- `src-tauri/src/scanner`: signature-driven discovery and size measurement.
- `src-tauri/src/safety`: planning, blacklist checks, and guarded tree deletion.
- `src-tauri/src/cleaner`: execution of verified generic filesystem plans.
- `src-tauri/src/large_files`: bounded traversal of approved user-content roots,
  streamed progress, file classification, and filesystem identity capture.
- `src-tauri/src/applications`: installed-app inventory plus constrained related
  Library-data inspection.
- `src-tauri/src/storage_commands`: IPC orchestration and ephemeral inventories
  for Large Files and App Uninstaller.
- `src-tauri/src/developer_artifacts`: explicit workspace registration,
  ecosystem-marker discovery, bounded candidate-tree measurement, progress and
  cancellation events, and private artifact inventory records.
- `src-tauri/src/trash_manager`: separate one-shot Trash planning and execution
  for user-reviewed files and apps.
- `src-tauri/src/docker` and `src-tauri/src/models_inventory`: domain adapters
  for resources that must not be treated as arbitrary files.
- `src-tauri/src/metrics` and `src-tauri/src/power`: platform system integration.
- `src-tauri/src/dev_ports`: bounded TCP-listener discovery, conservative
  development/testing-tool classification, opaque lease storage, TOCTOU validation,
  and exact-process graceful/force signaling.
- `src-tauri/src/ai_usage`: provider-specific usage collection and OAuth entry points.
- `src-tauri/src/agent_activity`: canonical project and Git worktree discovery, cross-tool AI agent process observation (8 adapters), truthful status matrix, dev port and cached artifact correlation, opaque 30s stop leases, and safe graceful termination with PID-reuse protection. Vendor hooks remain disabled until a verified event bridge exists. See [PROJECT_COCKPIT.md](PROJECT_COCKPIT.md).
- `src/lib/utils/tauri.ts`: frontend command wrappers — the single `invoke` boundary for all Tauri commands. The dedicated storage-management workflows (Large Files and App Uninstaller) are the sanctioned exception: their native and browser-preview split lives in `src/lib/api/storage.ts` and reuses the shared `isTauri()` from `src/lib/api/index.ts` to decide at runtime. New generic commands belong in `utils/tauri.ts`; new storage commands belong in `api/storage.ts`.
- `src/lib/stores`: Svelte state and lifecycle orchestration.
- `src/routes/dashboard` and `src/routes/quick`: the two window surfaces.
- `signatures`: reviewed cleanup definitions embedded in the Rust binary.

## Scan and cleanup flow

```text
persisted settings snapshot
   standard | intensive cleanup
                 |
                 v
registered signatures -- mode-aware filtering
        / domain adapters
                 |
                 v
          Rust ScanEngine
                 |
          ScanResult + scan_id
                 |
       selected item IDs only
                 v
          Rust SafetyPlanner
        validates the current scan,
        signature scope and risk
                 |
       private one-shot DeletePlan
          stored in PlanStore
                 |
       PlanPreview + opaque plan_id
                 |
          explicit user action
                 v
          Rust CleanExecutor
       expiry + identity + tree checks
```

`DeletePlan`, paths, strategies, and filesystem identities are Rust-private.
The frontend can request a plan only for item IDs in the current backend scan.
Plans expire after five minutes and are removed before execution, so they cannot
be replayed.

Generic cleanup supports only signature-scoped `Safe` and explicitly selected
`Rebuild` targets. `Manual` resources are rejected and must use a domain adapter.
For example, Ollama deletion resolves a freshly scanned model ID and calls
`ollama rm <model-name>`; it never deletes the manifest path supplied by a UI.

Read-only domain adapters may also add `Manual` observations to a scan without
granting deletion authority. The OrbStack adapter resolves only the reviewed
group-container VM disk path and reports its allocated blocks, not the sparse
file's logical capacity. It never enumerates other group containers, never
auto-selects the item, and the generic planner rejects the observation before
signature resolution.

### Standard and intensive scan scopes

Cleanup scope is a backend decision. `start_scan` snapshots the persisted
`intensive_cleanup` preference together with excluded signature IDs before
moving work to the blocking scanner thread. `SignatureRegistry` then omits
signatures marked `intensive_only` in standard mode and includes them in
intensive mode. The frontend cannot promote an individual signature into the
broader scope.

Intensive signatures remain declarative TOML entries and use the same scan,
planning, and execution pipeline as standard signatures. Broad cache and log
roots are never emitted as a single recursive target. The scanner considers
only their direct children, rejects symlinks and protected prefixes, and emits
a child only when the newest timestamp in its candidate tree satisfies the
signature's inactivity threshold. Cleanup repeats that tree-age check directly
before deletion.

The initial intensive scope covers stale third-party children of
`~/Library/Caches` and stale application-log groups under `~/Library/Logs`.
Apple/system cache namespaces and diagnostic/crash reports are excluded.
Temporary cleanup remains a separate known-prefix allowlist and never becomes
an unrestricted `/tmp` scan. Reviewed developer-tool prefixes still use the
same whole-tree inactivity threshold as every other temporary candidate.

See [SAFETY.md](SAFETY.md) for the full deletion contract.

## User-reviewed storage management

Large Files and App Uninstaller intentionally do not reuse `SignatureRegistry`,
`ScanEngine`, or generic `DeletePlan`. Those abstractions mean “Zenith has
classified this resource as disposable or rebuildable.” User files and inferred
app leftovers have a different trust model.

```text
Large Files / Applications UI
            |
       typed request
            v
backend-owned ephemeral inventory
            |
    opaque selected item IDs
            v
       TrashPlanner
 scope + identity captured in Rust
            |
  TrashPlanPreview + opaque UUID
            |
      explicit confirmation
            v
       TrashExecutor
 scope + type + identity revalidation
            |
        macOS Trash
```

Large Files only accepts the named user-content roots `Downloads`, `Desktop`,
`Documents`, and `Movies`. It does not follow symlinks, does not cross filesystem
boundaries, skips package directories such as `.app` and Photos libraries, and
keeps the 10,000 largest matches in a bounded result set while reporting
truncation to the UI. The generic cleanup blacklist deliberately protects
whole user-content directories, so Large Files uses its own narrower scope
predicate: a candidate must still be inside one of the approved roots and paths
containing `.git` remain protected. Symlinked roots are rejected, and every
component from the trusted root to the reviewed target is rechecked before
Trash. This exception does not widen generic cleanup authority.

The Large Files request also has a backend-validated filter. `all` keeps the
ordinary 100 MB minimum, while `installers` limits matches to `.dmg`, `.pkg`,
`.mpkg`, `.xip`, and `.iso` files and permits a 10 MB minimum. Installer results
are still user files: they are never auto-selected, never added to Quick Clean,
and are moved only through the same opaque one-shot Trash plan. The lower
threshold applies only after the root and extension allowlists have matched, so
it cannot turn the general Large Files scan into an unrestricted small-file
crawl.

App inventory scans only direct `.app` children of `/Applications` and
`~/Applications`. Related data is inspected only below an approved set of
`~/Library` roots. Exact bundle-identifier matches are high confidence; exact
app-name matches are medium confidence; Group Containers are treated as shared
unless exclusive ownership is proven. The app bundle itself is allowed through
the dedicated App Uninstaller scope even though `/Applications` is protected by
the generic blacklist.

Only the current app-uninstall inspection is retained. Execution rechecks that
the app is not running, moves the app bundle first, and skips all related data
if that first move does not succeed.

Both workflows use the native Trash adapter instead of permanent deletion.
Moving to Trash does not mean disk space has already been reclaimed; the UI
reports the amount moved and describes it as potentially reclaimable after the
Trash is emptied.

Developer Artifact Review is a third dedicated storage workflow. `Scan this
Mac` registers the canonical current-user home as a backend-owned scope, while
the native folder picker registers narrower user-owned workspaces. Both return
only opaque workspace IDs to the frontend. Whole-home discovery prunes system,
credential, media, package-manager state, and installed app-bundle trees before
recursion. Discovery recognizes generated trees only when direct project-root evidence proves their purpose: Cargo/Maven targets, Gradle
outputs, Node modules, Python environments, Composer/Ruby dependencies, Go,
.NET, CMake, Swift, Flutter, Elixir, and Terraform artifacts. Generic names
such as `build`, `vendor`, `bin`, or `target` are skipped without that evidence.
Ambiguous directories require additional generated evidence such as
`pyvenv.cfg`, Composer installation metadata, or `CMakeCache.txt`; ancestor
markers never authorize nested same-named source directories.

Discovery is cheap and does not descend into recognized artifact trees. The
independent candidate trees are measured with a small Rayon pool; each
candidate produces logical/allocated size, file count, and newest modification
time in one traversal. Results stream as measurements finish and cancellation
retains only individually completed candidates. Age is displayed as decision
metadata and never gates or auto-selects a candidate.

Cleanup accepts only opaque artifact IDs from a fresh inventory. The planner
captures workspace/project/marker identities and the exact relative artifact
type. Trash execution revalidates those identities, marker evidence, scope,
symlink components, directory type, and completeness immediately before each
move. Only the exact reviewed generated directory moves to Trash; source files,
manifests, lockfiles, project roots, and workspace roots remain in place.
Developer artifacts never contribute to Quick Clean totals.

## Concurrency and lifecycle

Filesystem scan and cleanup share a Rust operation mutex. This prevents the two
WebViews from starting overlapping mutations or duplicate scans against shared
state. Large-file traversal, app inventory/inspection, and Trash execution use
the same storage-operation serialization. Blocking filesystem, process, CLI,
and synchronous HTTP work runs outside the async command thread.

The quick window is persistent but inactive while hidden:

- opening reloads persistent preferences and displays cached data first;
- disk metrics refresh once per activation;
- memory polling starts only while visible and stops when hidden;
- AI usage uses a 60-second backend cache;
- scan data is reused until stale;
- Escape, Cmd+W, focus loss, and the close button hide rather than quit.

Store constructors do not start I/O. A route or an explicit activation event
owns refresh and cleanup of recurring work.

Polling stores use reference-counted subscribers: the first subscriber starts
the timer and the last subscriber stops it. Repeated starts are idempotent, one
consumer cannot stop another consumer's polling, and fake-timer tests verify
the lifecycle without wall-clock sleeps. Backend cancellation registries are
similarly lifecycle-owned: entries expire after a TTL, are capped at 64, and
are removed after success, cancellation, or scanner failure.

Development-port discovery runs independently from the 2.5-second memory
sampler. The standalone Development Servers route refreshes development and
verified local testing-tool listeners at a
slower interval only while visible, prevents overlapping discovery calls, and
moves all blocking `lsof`, process-snapshot, wait, and signal work onto the
blocking runtime. Existing dashboard settings receive the new tab once after
Memory; later visibility and ordering choices remain user-controlled.

Project Cockpit has a separate, read-only `agent_activity` domain. A bounded
10-second Rust cache owns `ProjectContextSnapshot` / `AgentActivitySnapshot`,
which is the canonical project/session input for the paired AI Control Center.
The command runs process and filesystem inspection through `spawn_blocking`;
the Svelte route refreshes only on mount or explicit user action and never owns
a timer. The Quick Panel has no permission for this command.

Classification requires an exact adapter executable basename, current UID,
non-zero process start time, executable path, and safe cwd evidence. Repository
correlation walks cwd ancestors to the deepest `.git` marker and distinguishes
linked worktrees from ordinary repositories. Canonical paths stay backend-only:
SHA-256-derived opaque IDs and a compact parent/name hint cross IPC, while PID,
argv, environment, Git remotes, file changes, and full paths do not. When cwd
cannot be verified, the session remains explicitly Unassigned.

## AI Control Center

AI Control Center (`src-tauri/src/ai_control_center`) provides a unified,
provenance-aware local control plane:

- **Observation provenance:** Provider observations carry an explicit source
  kind (`LiveAuthoritative`, `LiveQuota`, `LocalEstimate`, `Manual`), scope
  (`Subscription`, `ApiKey`, `Project`, `Organization`, `LocalSessions`), and
  quality (`Fresh`, `Stale`, `Partial`, `Unavailable`). Authoritative billing,
  local estimates, and manual values are never conflated.
- **Shared session dependency:** Consumes the canonical `AgentActivityRegistry`
  (`snapshot` and `project_roots`) from Project Cockpit. It does not run a
  competing process classifier or rely on CWD authority alone.
- **Policy engine:** Evaluates memory pressure, battery transitions, session
  exits, orphan processes, dev ports, and cleanup opportunities in Rust.
  Recommendations are advisory-first; native macOS notifications are emitted
  only for explicitly enabled user preferences with cooldown deduplication.
- **Opaque action previews:** Recommendations generate opaque, expiring (120s),
  one-shot `RecommendationPreview` tokens. Consuming a preview directs the user
  to dedicated workflows (such as Development Servers or Developer Artifacts);
  it never performs destructive mutations directly.
- **Safety posture:** User-initiated, bounded scan (max 2,000 files, 1 MiB per
  file, depth 8) of registered active project roots. Secret and MCP/permission
  findings are sanitized before crossing IPC; symlinks are never followed and
  config files are never executed.
- **Git baseline tracking:** Captures a repository baseline on first session
  observation. Post-baseline modifications are tracked as metadata counts; full
  diff content is fetched on-demand, bounded to 256 KiB, and never persisted.
- **Quick-panel and cache lifecycle:** The quick panel reads only the last
  cached `ControlCenterQuickSummary` in-memory. Hidden panels execute zero
  provider calls, Git commands, or safety scans. Full snapshots use a bounded
  backend cache protected by an async refresh lock.

## Settings

Preferences are validated in Rust and stored at the Tauri application config
directory as `settings.json`. Writes use a temporary file followed by rename so
an interrupted save does not replace the last valid configuration. Missing
fields receive safe defaults during upgrades.

The stored settings drive theme, quick-panel section/provider order, Keep Awake
rules, excluded signatures, Quick Clean category defaults, and the opt-in
intensive scan scope. `intensive_cleanup` defaults to `false`, including when an
older settings file does not contain the field. Launch-at-login is deliberately
marked as unavailable until a native autostart integration is implemented.

## IPC security

Tauri capabilities are split by window:

- `capabilities/quick.json` grants read-oriented commands plus backend-owned
  safe plan creation/execution.
- `capabilities/main.json` additionally grants model deletion, Docker pruning,
  process termination, settings writes, power controls, Large Files inspection,
  app inspection, dedicated Trash-plan execution, and development-listener
  inspection/release. Development-port permissions are intentionally absent
  from the quick panel.

The global Tauri JavaScript object is disabled and a Content Security Policy is
applied in development and production. Adding a command requires all three:
registration in `lib.rs`, declaration in `build.rs`, and an intentional window
capability entry.

### IPC numeric safety contract

Zenith binds Rust structs to TypeScript via Tauri Specta using
`dangerously_cast_bigints_to_number()`. Every serialized `u64` and `Option<u64>`
field uses the shared `ipc_numeric` serde boundary. Values up to JavaScript
`Number.MAX_SAFE_INTEGER` ($2^{53} - 1$) round-trip as numbers; larger values are
rejected during serialization or deserialization instead of being rounded.
The paired Specta annotation tells binding generation that the wire type remains
TypeScript `number`. Boundary tests exercise both the shared serializer and real
IPC model payloads.

### Browser-preview contract

Browser preview is an alternate transport for the same typed frontend API, not
a second implementation of backend policy. Domain fixtures live under
`src/lib/api/mocks`, return fresh value copies, and preserve native response and
error shapes. Contract tests compare the exact top-level keys of native and mock
APIs, including the dedicated storage workflow surface, so adding a native
method requires an intentional mock decision.

### CI dependency graph

The shared Linux frontend job exports Specta bindings, checks binding and lock
file drift, runs Svelte/Vitest, builds `dist`, and uploads that verified frontend
artifact. macOS and Windows x64 run Rust format, Clippy, tests, and check in
parallel. Each packaging smoke job depends on the shared frontend artifact and
its matching Rust job, proving that the platform bundle embeds the exact tested
frontend without rerunning the same frontend suite on every OS. Both packaging
jobs pass the checked-in `.github/tauri.package-ci.json` override by path; this
avoids shell-specific inline JSON quoting and disables `beforeBuildCommand`.

### Release dependency graph

The release workflow is intentionally a fan-out/fan-in pipeline:

```text
version + binding + frontend verification
              |
       +------+------+
       |             |
 macOS ARM64     Windows x64
 unsigned DMG    current-user NSIS
       |             |
       +------+------+
              |
     one tagged prerelease
```

Only the final job has `contents: write`; platform jobs can build and upload
workflow artifacts but cannot create competing GitHub Releases. Public filenames
are stable, while their download URLs remain immutable because the version is
part of the tag path. Each platform emits separate build metadata and checksums,
and the publisher also emits their combined checksum file.

The Windows job generates a WinGet community-repository multi-file manifest
from the exact NSIS bytes and computed SHA256 hash. v0.2.0 is the explicitly
unsigned transition release. After SignPath Foundation approval, signing must
be inserted between build and checksum generation and must follow
`CODE_SIGNING_POLICY.md`; WinGet submission remains a post-publication gate so
its immutable URL can be validated in Windows Sandbox.

## External tools

On macOS and Windows, applications launched from the desktop shell receive a
distinct `PATH` compared to an interactive shell. `tooling.rs` resolves CLIs
through inherited paths and standard platform locations (Homebrew, local AppData,
Program Files, Docker, and Ollama) before spawning processes. Adapters fail closed
when a required tool is unavailable.
