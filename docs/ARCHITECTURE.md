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
        commands/ (Tauri adapters: decode, authorize, one use case, DTO)
                    |
                    v
        DesktopState (bounded services + platform facts)
        +-----------+-----------+-------------+
        |           |           |             |
     cleanup      storage       ai          system
   CleanupService StorageService AiService  SystemService
        |           |           |             |
     scanner +    reviewed     provider     metrics/power
     safety +     storage      usage +      memory, disk,
     plan store   workflows    activity +   Keep Awake,
                               AI control   dev ports
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
coordination therefore live in the Rust application services — `CleanupService`,
`StorageService`, `AiService`, and `SystemService`, built by the composition
root — rather than in a browser singleton. Lifecycle-owned storage state uses
poison recovery for bounded inventories, plans, workspace registrations, and
cancellation handles; the settings authority and destructive authority fail
closed when their state cannot be trusted.

### Tauri is an adapter, not the application core

`src-tauri/src/composition` is the only layer that names every concrete
implementation: the environment the backend is described by, the catalog a scan
uses, the native adapters behind the ports, and the shared handles two services
deliberately hold together. `DesktopState` declares what a command may reach —
platform facts, the settings authority, and the four bounded services — and no
scan, inventory, plan store, or workflow mutex is part of it.

A Tauri command does five things and nothing else: it decodes IPC input,
resolves what only the desktop shell can resolve (a window boundary, an app
config directory), constructs the Tauri-specific adapter for that call
(`crate::events`: a `Channel` sink, a notification port), calls one application
use case, and maps the typed result back to an IPC DTO. It never locks a cache,
applies a plan TTL, selects cleanup candidates, calls the safety planner or the
clean executor, or mutates the filesystem.

Services emit domain events and state what happened; `crate::events` is the only
module that knows the transport is a Tauri `Channel` or the notification plugin.
`src-tauri/src/blocking` is the same seam for work that must leave the command
executor: commands and services both hand blocking work to it, so a worker that
died reports what it died with through one redacting sink.

Ask three questions of new code:

```text
Does it depend on Tauri, a WebView, the tray, or window lifecycle?
    yes -> src-tauri (adapter); commands stay thin, transport lives in events/
Does it implement a native OS capability?
    yes -> crates/zenith-platform, behind a port
Does it express Zenith product semantics or a use case?
    yes -> crates/zenith-core, or an application service in src-tauri/src/services
```

## Platform capability contract

Platform-specific behavior is selected behind Rust service boundaries instead
of being spread through route components. `crates/zenith-platform` owns the
runtime platform contract — native probing, path resolution, process control,
system actions, atomic file replacement, and the Trash adapter — and its
`PlatformCapabilitiesProvider` exposes a typed, read-only snapshot through
`get_platform_capabilities`. The frontend loads the snapshot independently in
each WebView and uses it to hide or disable actions that are unavailable on the
current platform. A `read_only` capability may continue to expose inspection
metrics, but mutating controls require an `available` capability.

The crate depends on `zenith-core` and never on `zenith-desktop` or Tauri, so
the same native probing serves a scan, an application service, or a future CLI.
An answer that comes from another layer arrives as a probe instead of a call:
the container-CLI question is asked where tool resolution lives and injected
into the capability provider, so the snapshot cannot disagree with the adapter
that drives the CLI.

The boundary is ownership, not "no OS call outside this crate". Native code
that belongs to another bounded context stays with its owner in the desktop
crate: keychain and Windows Credential Manager access in
`ai_providers::credentials`, the handle-level identity and deletion-safety
primitives in `safety`, Windows allocated-size measurement in `scanner::size`,
and process/machine introspection in `metrics`, `power`, `dev_ports`,
`process_owner`, and `diagnostics`. Moving one of those is its own change.

Both macOS and Windows x64 implement the capability contract across all thirteen
core features. Platform-specific actions, including workspace selection, use
native adapters tailored to each OS; unsupported platforms report unavailable
capabilities instead of exposing nonfunctional controls.

Workspace selection uses `NSOpenPanel` on macOS and the Windows Shell COM folder
picker through a static PowerShell script on Windows. The Windows adapter never
interpolates user-controlled text into the script, requests UTF-8 output, and
maps picker cancellation to `None` just like the macOS adapter.

## Crate boundary

Zenith is a Cargo workspace with three members:

```text
Cargo.toml             workspace manifest: version, edition, MSRV, release profile
crates/zenith-core     product semantics, with no desktop framework in the graph
crates/zenith-platform native macOS/Windows integration behind narrow ports
src-tauri              zenith-desktop: the Tauri adapter and the `Zenith` binary
```

Every file in `zenith-core` answers one question the same way: *would this
still make sense if Zenith had a CLI instead of a Tauri window?* Scanning,
cleanup safety, storage policy, platform capability description, and the DTOs
the interface is allowed to see all say yes. Webview IPC, tray and window
lifecycle, capability grants, and desktop composition all say no, so they stay
in `zenith-desktop`.

`zenith-platform` answers the same question for the other kind of coupling.
Reading a Windows registry value, resolving a known folder, terminating a
process tree, revealing a path in Finder, replacing a file atomically, or moving
a reviewed file to the Trash is native work that a CLI would still need, so it
lives there behind ports — `PlatformCapabilitiesProvider`,
`PlatformPathsProvider`, `SystemActionProvider`, the process-control functions,
and `TrashBackend`. The crate depends on `zenith-core` and never on
`zenith-desktop`, so a native call cannot start depending on a window. Its
dependency direction is enforced by `scripts/check_core_boundaries.cjs` together
with the rest of the boundary rules below.

Windows behavior keeps the shape the rest of the tree uses: rules that can be
written as pure functions over an explicit `PathFlavor` are, so a macOS runner
asserts the same Windows rule a Windows runner does, and the crate's own test
suite runs on every host. OS entry points that cannot be written that way stay
`#[cfg]`-gated inside the module that owns them rather than in a second
module-per-OS tree that only one runner compiles.

Inside the domain crate the split is by responsibility, not by screen:

- `domain/scan`: what was measured, and whether it may be cleaned.
- `domain/cleanup`: what may be deleted. `DeletePlan` and `DeleteTarget` are
  authorization state and carry no `serde` or `specta` derives; the only way a
  plan reaches the interface is `application/dto/cleanup`, whose
  `DeletePlan::preview` projection drops paths, strategies, and captured
  identities by construction rather than by an attribute someone could remove.
- `domain/storage`: the vocabulary and thresholds of the storage workflows.
- `domain/platform`: the capability snapshot and the per-platform vocabulary.
- `domain/risk`, `domain/identity`, `domain/paths`, `domain/observation`:
  shared primitives — risk tiers, filesystem identity, path invariants, and
  observation quality.
- `application/dto`: serializable projections, grouped by the workflow that
  produces them.

`domain/identity` separates two concepts that look alike and must not be
interchangeable. `FileIdentity` is the device and inode a path resolved to.
`CleanupIdentity` is what generic cleanup captured at plan time — entity, file
type, size, and a sub-second modification stamp — while
`ReviewedFileIdentity` is what a reviewed storage workflow captured about a
user-selected target. Because they are different types, a reviewed Trash target
cannot satisfy a cleanup TOCTOU check.

The boundary is enforced rather than documented.
`scripts/check_core_boundaries.cjs` reads the resolved dependency graph from
`cargo metadata` and fails when `zenith-core` declares `tauri`, `tauri-build`,
`tauri-plugin-*`, `windows-sys`, `security-framework`, or `rfd` as any kind of
dependency, or reaches a `tauri*`, `windows*`, `security-framework*`, or `rfd`
crate over normal and build edges at any depth. Both `zenith-core` and
`zenith-platform` additionally refuse `zenith-desktop`: dependencies point into
the domain, never back out of it, so an edge up to the crate that owns the
window inverts the layering and is deleted rather than moved.
`just check-architecture` runs that check together with `cargo check` of both
crates; CI runs it on macOS and Windows.

The execution authority that used to be missing now lives where it is used:
`src-tauri/src/safety` owns the validated authority values, and the ports the
platform layer implements are declared in `crates/zenith-platform` itself,
because no other crate has to name them.

## Repository map

- `crates/zenith-core/src/domain`: product semantics that must not depend on the
  desktop framework — risk, scan vocabulary and cleanup eligibility, cleanup
  authorization, storage policy, platform capabilities, filesystem identity,
  path invariants, and observation quality.
- `crates/zenith-core/src/application/dto`: the serializable projections the
  interface sees — plan previews, clean results and events, scan events, and the
  storage-management inventories.
- `src-tauri/src/models`: the desktop model surface. It re-exports the domain
  semantics above by name and adds the DTOs only the desktop adapter produces
  (AI usage, metrics, Docker, keep-awake, ports, agent activity, developer
  artifacts, diagnostics, settings), so command modules keep one import root.
- `src-tauri/src/commands`: the Tauri IPC boundary. `mod.rs` only composes
  domain exports; `ai.rs`, `cleanup.rs`, and `system.rs` own handlers, and
  `state.rs` declares `DesktopState` — the bounded services and platform facts a
  handler may reach. No handler locks a cache, stores a plan, or mutates a file.
- `src-tauri/src/composition`: the composition root. It builds the environment,
  the catalog, the shared handles, and the four services, and it is the only
  place that binds a port to a concrete native or Tauri adapter.
- `src-tauri/src/events`: Tauri transport adapters. Scan, cleanup,
  reviewed-storage, and provider progress become `Channel` sinks here, and the
  desktop-notification port is implemented here, so a service can be exercised
  without a Tauri runtime.
- `crates/zenith-platform`: the native platform layer. `description.rs` holds
  the injectable `PlatformEnvironment` description, `environment.rs` probes the
  running machine, `paths.rs` resolves user roots and known folders,
  `path_algebra.rs` owns the flavor-parameterized path rules, `process.rs` and
  `subprocess.rs` own process termination and bounded child execution,
  `system_actions.rs` opens and reveals paths, `file_ops.rs` replaces files
  atomically, `capabilities.rs` composes the capability snapshot, and `trash.rs`
  is the only place the `trash` crate is named.
- `src-tauri/src/scanner`: signature-driven discovery and size measurement. The
  traversal owns symlink policy, depth bounds, error classification,
  measurement completeness, and cancellation; progress and cancellation arrive
  as `zenith-core` contracts, never as framework types.
- `src-tauri/src/safety`: planning, blacklist checks, validated authority
  (`ValidatedTarget`, `ValidatedModelTarget`, `FilesystemDeleteAuthority`), and
  guarded tree deletion. The authority types have no public constructor outside
  the safety layer.
- `src-tauri/src/services`: the application services. `CleanupService` owns the
  cleanup lifecycle (gate, budgets, plan store, scan store, scan invalidation)
  for Main Clean and Quick Clean alike, `StorageService` owns the
  reviewed-storage lifecycle (gate, budgets, ephemeral inventories,
  cancellation registries, reviewed workspaces, plan store, Trash executor) for
  Large Files, Developer Artifact Review, and App Uninstaller, `AiService` owns
  provider credentials, the usage and agent-activity caches with their
  single-flight and generation contracts, the Control Center state, and the
  background runtime, `SystemService` owns memory and disk observations, Docker
  status and pruning, local-model inventory and deletion, Keep Awake,
  development-port leases, diagnostics, and the settings authority's
  persistence path, `ScanService` runs a framework-free scan, and `PlanStore`
  bounds both cleanup and reviewed Trash plans under one lifecycle.
- `src-tauri/src/cleaner`: execution of verified plans. Each target is
  classified into `CleanupOperation` first, so only a filesystem operation can
  reach the validated deletion primitive.
- `src-tauri/src/large_files`: bounded traversal of approved user-content roots,
  streamed progress, file classification, and filesystem identity capture.
- `src-tauri/src/applications`: installed-app inventory plus constrained related
  Library-data inspection.
- `src-tauri/src/storage_commands`: thin IPC adapters for the reviewed-storage
  workflows. The inventories, cancellation handles, workspace registry, gate,
  budgets, and plan store belong to `services::StorageService`.
- `src-tauri/src/developer_artifacts`: explicit workspace registration,
  ecosystem-marker discovery, bounded candidate-tree measurement, progress and
  cancellation events, and private artifact inventory records.
- `src-tauri/src/trash_manager`: separate one-shot Trash planning and execution
  for user-reviewed files and apps. It owns `ApprovedTrashTarget`, the only
  value the Trash port accepts, and each scope's evidence requirements.
- `src-tauri/src/docker` and `src-tauri/src/models_inventory`: domain adapters
  for resources that must not be treated as arbitrary files.
- `src-tauri/src/metrics` and `src-tauri/src/power`: platform system integration.
  The Memory view reports what one process-table snapshot observed. A process
  group is labelled as started by Zenith only when that same snapshot traces
  every member's ancestry to this process; everything else is reported as
  observed, with the parent names the snapshot resolved. Termination eligibility
  is a separate, allowlist-gated question and never implies Zenith launched the
  process.
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
be replayed. `CleanupService` owns that lifecycle end to end; the IPC layer
submits a scan ID, selected item IDs, and an opaque plan ID and nothing else.

Execution classifies each target into the operation it authorizes before
anything mutates:

```text
DeleteTarget.strategy
        |
        v
  CleanupOperation::of
   /        |         \
Filesystem Container  Provider
   |          |          |
   |          |          +-- the tool prunes its own cache with fixed
   |          |              arguments; the planned location is only a
   |          |              staleness assertion
   |          +------------- the container runtime prunes what it owns;
   |                        a `docker://` pseudo path carries no authority
   +------------------------ revalidate, then delete through the safety
                            layer's validated authority
```

Only the `Filesystem` operation names a path as mutation authority, and it is
the only one whose strategy can reach `SafeTreeDeleter`. A `Manual` target
classifies to no operation at all and is refused rather than falling back to a
filesystem mutation.

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
Apple/system cache namespaces and diagnostic/crash reports are excluded, and a
prefix exclusion matches case-insensitively because a cache namespace's on-disk
casing is not stable. A namespace whose owner publishes its own invalidation
command is excluded rather than treated as a generic cache, so `dotslash` and
Playwright's `ms-playwright` downloads remain owned by their CLIs. Temporary
cleanup remains a separate known-prefix allowlist and never becomes an
unrestricted `/tmp` scan. Reviewed developer-tool prefixes still use the same
whole-tree inactivity threshold as every other temporary candidate.

One predicate decides what a scan offers and what every surface counts. Items
whose observation cannot support a cleanup contribute zero bytes to the risk
buckets, the category totals, the scan totals, and the selection summary, so a
category total always equals the sum of its buckets and an inaccessible item is
never selectable. A completed partial scan is explained once, through the
item's own observation quality and the scan's durable `incomplete_reasons` and
counters — never through a second, destructive error surface alongside a scan
that otherwise succeeded.

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
   Native Trash / Recycle Bin
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

The Trash adapter is a port. `zenith_platform::TrashBackend` is the only place
the `trash` crate is named, the composition root hands the adapter to
`StorageService` so the production path and a test use the same code, and the port accepts a `ReviewedTrashEntry` —
a type only the reviewed-storage layer mints, immediately after scope, identity,
symlink, and evidence checks pass — rather than a bare path. Reviewed Trash
plans live in the same bounded, expiring, one-shot store as cleanup plans, so
TTL, capacity, and replay refusal are one implementation instead of two. Nothing
in this workflow feeds Quick Clean: its candidates come only from a completed
`ScanResult`.

Developer Artifact Review is a third dedicated storage workflow. `Scan this
computer` registers the canonical current-user home as a backend-owned scope, while
the native folder picker registers narrower user-owned workspaces. Both return
only opaque workspace IDs to the frontend. Whole-home discovery prunes system,
credential, media, package-manager state, and installed app-bundle trees before
recursion. `Downloads` stays in scope: users keep projects there. Because macOS
gates that folder behind a user consent prompt that parks the reading thread,
the scan probes the folder **before** it takes the storage-operation gate, so a
waiting dialog cannot stall the mutating storage commands queued behind that
gate. The resolved answer is passed into the walk, which then neither prompts a
second time nor blocks on the same folder again.

A folder the walk cannot read is recorded by path with its reason and whether a
retry can include it, reported while the scan runs and again in the final
result. A refused `Downloads` therefore surfaces as an uninspected entry with a
rescan affordance instead of an anonymous skip counter, and the scan completes
as partial rather than appearing to have covered everything. Discovery recognizes generated trees only when direct project-root evidence proves their purpose: Cargo/Maven targets, Gradle
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

Ownership of that serialization is deliberate. `CleanupService` owns the
operation gate, the execution budgets, the plan store, and the scan store for
the cleanup lifecycle, and `StorageService` owns the same gate and budgets
alongside the reviewed-storage inventories, cancellation registries, workspace
registry, plan store, and Trash executor. Their methods acquire the gate
themselves, so no command handler decides when a mutation may start:
`CleanupService::{start_scan, create_delete_plan, execute_clean,
quick_clean_safe}` and `StorageService::{scan_large_files,
scan_developer_artifacts, installed_apps, inspect_app_uninstall,
execute_trash_plan}` take it inside the service.

`StorageOperationGate` and `ExecutionBudgets` are `Clone` over one `Arc`, so
every workflow shares the same lock and the same permits rather than holding a
second, independent one. The gate is not re-acquired by lower-level helpers:
`CleanExecutor`, `SafeTreeDeleter`, and the provider adapters mutate only after
their caller has taken a write permit, and the reviewed-storage scanners run
inside a read permit their service acquired.

One-shot plans — delete plans and reviewed Trash plans — live in the same
bounded store. It owns TTL expiration, bounded capacity with oldest-first
eviction, stale-plan rejection, and removal on consumption, so a workflow
cannot drift by editing a map beside its commands. A plan is removed when it is
taken, whether the execution then succeeds or fails, so a plan ID authorizes at
most one mutation.

A scan reports progress through a `ScanProgressSink` and answers cancellation
through a `CancellationProbe`, both of which live in `zenith-core`; the Tauri
`Channel` is only the outermost adapter. Cancellation is checked at category
boundaries, before each signature, and inside the traversal — at each directory
boundary and entry of a measured tree. A cancelled walk stops at the next
boundary, keeps what it already observed, and reports an incomplete
measurement, so the retained bytes are a lower bound and the item is never
auto-selected. The execution-time tree-age re-check is not cancellable: it must
observe the whole tree before a deletion is allowed.

The quick window is persistent but inactive while hidden:

- opening reloads persistent preferences and displays cached data first;
- disk metrics refresh once per activation;
- memory polling starts only while visible and stops when hidden;
- AI usage uses a 60-second backend cache, auto-refreshed while its tab stays
  visible via a ref-counted subscriber; failures stay manual;
- scan data is reused until stale; visible surfaces auto-rescan within ~1s of
  expiry instead of waiting for a manual click, failed scans stay manual, and
  hidden panels never scan;
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

- `capabilities/quick.json` grants read-oriented commands plus the
  backend-owned safe cleanup intent (`quick_clean_safe`, which selects the
  backend's own Safe-tier candidates and cannot be pointed at a target).
- `capabilities/main.json` additionally grants model deletion, Docker pruning,
  process termination, settings writes, power controls, Large Files inspection,
  app inspection, dedicated Trash-plan execution, and development-listener
  inspection/release. Development-port permissions are intentionally absent
  from the quick panel, and the reviewed set is asserted by the contract test
  below.

`src-tauri/tests/capability_contract_tests.rs` holds the split to a contract
rather than a convention: every grant must name a permission the build
generates, every registered command must be granted to the main window, and the
quick window must equal a reviewed read-mostly allowlist that contains no
mutating command. A grant the panel does not need fails the suite until it is
written into that allowlist, which is where the review happens.

Capability files are an IPC boundary, not the authorization model. They decide
which window may call a command; the application services still enforce the
trusted inventory, scan freshness, opaque IDs, plan TTLs, one-shot execution,
signature scope, captured identity, and TOCTOU checks on every call, exactly as
they do for the dashboard. A compromised renderer that can call an allowed
command cannot convert it into arbitrary filesystem authority, because no
command accepts a path, a strategy, or an identity from the interface.

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
parallel. Two additional jobs are independent of the packaging chain: `msrv`
builds with the toolchain declared in the root `Cargo.toml`
`[workspace.package]` table (Rust 1.95.0) through `just check-msrv`, and
`supply-chain` runs `just supply-chain`
(`cargo deny`, `cargo audit`, and `pnpm audit`) against both lockfiles. Each
packaging smoke job depends on the shared frontend artifact and its matching
Rust job, proving that the platform bundle embeds the exact tested frontend
without rerunning the same frontend suite on every OS. Both packaging jobs pass
a checked-in `.github/tauri.package-ci.json` override by path; this avoids
shell-specific inline JSON quoting and disables `beforeBuildCommand`. The
Windows packaging job builds both install modes and runs the installer/doctor
gate described under [Release dependency graph](#release-dependency-graph).

### Release dependency graph

The release workflow is intentionally a fan-out/fan-in pipeline:

```text
version + binding + frontend verification
   + SPDX SBOM (locked manifests)
              |
       +------+------+
       |             |
 macOS ARM64     Windows x64
 unsigned DMG    per-user + machine-wide NSIS
       |             |
       +------+------+
              |
   build provenance attestation
   (id-token + attestations write)
              |
   release-approval environment:
   record + verify endpoint-review.json
              |
     one tagged prerelease
```

Only the final job has `contents: write`. The attestation job carries only
`id-token: write` and `attestations: write`, and platform jobs can build and
upload workflow artifacts but cannot create competing GitHub Releases. Public
filenames are stable, while their download URLs remain immutable because the
version is part of the tag path. Each platform emits build metadata and a
checksum manifest; the verification job emits one SPDX SBOM generated from the
locked `Cargo.lock`, `package.json`, and `pnpm-lock.yaml` (a staging directory
keeps the scanner out of the multi-gigabyte build tree), and the publisher
hashes the SBOM into its own manifest and emits the combined checksum file. All
of them go through `scripts/release_checksums.cjs` rather than shell-specific
text writers. The publisher normalizes any incoming CRLF to LF, validates every
checksum line, and runs `shasum -c` against the merged artifacts before it can
create a release.

The Windows job builds the per-user installer from `src-tauri/tauri.conf.json`
and the machine-wide installer from `.github/tauri.nsis-permachine.json`, then
generates the WinGet community-repository multi-file manifest from the per-user
installer's exact NSIS bytes and computed SHA256 hash. The public release
remains unsigned in this transition; after SignPath Foundation approval,
signing must be inserted between build and checksum generation and must follow
`CODE_SIGNING_POLICY.md`, with checksums, SBOM, and attestation computed from
the verified signed bytes. WinGet submission remains a post-publication gate so
its immutable URL can be validated in Windows Sandbox.

Two gates run before publication. The `attest-provenance` job uses
`actions/attest-build-provenance` to bind each installer and SBOM to the
repository, workflow, and tagged commit. The publishing job then runs inside the
`release-approval` environment, where a maintainer submits the installers to
Microsoft's endpoint-protection analysis and records the result; a detection, a
missing record, or an artifact whose hash differs from the reviewed bytes fails
the release before the publish step. `scripts/endpoint_review.cjs` writes and
verifies the machine-checkable record.

Windows packaging is also gated in CI: the `package-windows` job installs the
built per-user installer silently, runs the binary's `--doctor` self-check,
requires exit code 0 with zero failing checks, uninstalls silently, and fails if
the install directory survives. The same job proves the machine-wide installer
packages.

## External tools

On macOS and Windows, applications launched from the desktop shell receive a
distinct `PATH` compared to an interactive shell. `tooling.rs` resolves CLIs
through inherited paths and standard platform locations (Homebrew, local AppData,
Program Files, Docker, and Ollama) before spawning processes. Adapters fail closed
when a required tool is unavailable. Resolved background commands and direct
commands managed by the timeout helper set Windows `CREATE_NO_WINDOW`, including
native picker adapters; actions whose purpose is to open a terminal bypass this
helper.

Windows `std::fs::canonicalize` returns verbatim paths such as `\\?\C:\...`.
Safety comparisons normalize the verbatim drive or UNC prefix before applying
drive-root, protected-directory, traversal, and alternate-data-stream rules.
Backend records may retain canonical paths for filesystem identity and long-path
operations, but serialized display paths must use the normalized form. The
automatic AI safety scanner applies the same normalization and resolves
`USERPROFILE` before `HOME` on Windows so drive roots, the user profile, and
broad operating-system roots can never become recursive scan scopes.

Cross-platform system actions use capability and IPC names that describe the
intent (`open_storage_settings`, `show_in_file_manager`) rather than a specific
macOS application. Native adapters remain responsible for choosing the platform's
file manager and storage settings pane at runtime.

### Cache provider registry

Cache coverage is split by ownership, not by display language. Stable,
independently rebuildable user-cache directories use platform-scoped TOML
signatures. Stores with owner-provided locking or garbage collection use the
typed cache-provider registry. Mixed, relocated, WSL/container, model, and
application-configured roots are advisory until a dedicated adapter can prove
their identity and scope. See [CACHE_SUPPORT.md](CACHE_SUPPORT.md).

Provider ScanItems carry source, management mode, artifact role, rebuild
consequence, and physical-byte confidence. The generic scan remains one UI and
one opaque plan flow, but `external_command` dispatches only to a registered
provider; it is never an alias for recursive deletion. Provider discovery and
mutation use backend-owned fixed argv and fresh cache-path validation. A failed
or missing CLI degrades locally and does not fail unrelated signatures.
