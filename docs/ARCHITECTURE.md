# Architecture

Zenith is a macOS menu-bar application built with Tauri 2, Rust, and Svelte 5.
Rust owns system access and destructive decisions. Svelte renders typed state and
submits user intent; it does not construct filesystem operations.

## Runtime shape

```text
macOS menu bar
     |
     +-- quick WebView (hidden until requested)
     |      read metrics, scan, execute a backend-owned safe plan
     |
     +-- main WebView
            dashboard, settings, reviewed destructive adapters
                    |
                    | typed Tauri IPC
                    v
              Rust application core
        +-----------+-----------+-------------+
        |           |           |             |
     scanner      safety      adapters      metrics/power
        |        planner +     Docker,       memory, disk,
   signatures    plan store    models, AI    Keep Awake
        |
        +-- dedicated storage management
            large-file inventory, app inventory,
            one-shot Trash plans

        +-- development-port management
            listener discovery + classification,
            one-shot leases, exact-process signaling
```

The windows have separate frontend runtimes and stores. Shared authority and
coordination therefore live in Rust `AppState`, backend-owned inventories, or
Rust process state rather than in a browser singleton.

## Repository map

- `src-tauri/src/commands`: narrow IPC boundary and shared application state.
- `src-tauri/src/scanner`: signature-driven discovery and size measurement.
- `src-tauri/src/safety`: planning, blacklist checks, and guarded tree deletion.
- `src-tauri/src/cleaner`: execution of verified generic filesystem plans.
- `src-tauri/src/large_files`: bounded traversal of approved user-content roots,
  streamed progress, file classification, and filesystem identity capture.
- `src-tauri/src/applications`: installed-app inventory plus constrained related
  Library-data inspection.
- `src-tauri/src/storage_commands`: IPC orchestration and ephemeral inventories
  for Large Files and App Uninstaller.
- `src-tauri/src/trash_manager`: separate one-shot Trash planning and execution
  for user-reviewed files and apps.
- `src-tauri/src/docker` and `src-tauri/src/models_inventory`: domain adapters
  for resources that must not be treated as arbitrary files.
- `src-tauri/src/metrics` and `src-tauri/src/power`: macOS system integration.
- `src-tauri/src/dev_ports`: bounded TCP-listener discovery, conservative
  development-server classification, opaque lease storage, TOCTOU validation,
  and exact-process graceful/force signaling.
- `src-tauri/src/ai_usage`: provider-specific usage collection and OAuth entry
  points.
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
an unrestricted `/tmp` scan.

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

Development-port discovery runs independently from the 2.5-second memory
sampler. The Memory route refreshes listeners at a slower interval only while
visible, prevents overlapping discovery calls, and moves all blocking `lsof`,
process-snapshot, wait, and signal work onto the blocking runtime.

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

## External tools

Finder-launched macOS applications often receive a smaller `PATH` than an
interactive shell. `tooling.rs` resolves CLIs through the inherited path and
common Homebrew, local-user, Docker, and Ollama locations before spawning them.
Adapters still fail closed when a required tool is unavailable.
