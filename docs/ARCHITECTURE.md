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
```

The windows have separate frontend runtimes and stores. Shared authority and
coordination therefore live in Rust `AppState`, not in a browser singleton.

## Repository map

- `src-tauri/src/commands`: narrow IPC boundary and shared application state.
- `src-tauri/src/scanner`: signature-driven discovery and size measurement.
- `src-tauri/src/safety`: planning, blacklist checks, and guarded tree deletion.
- `src-tauri/src/cleaner`: execution of verified generic filesystem plans.
- `src-tauri/src/docker` and `src-tauri/src/models_inventory`: domain adapters
  for resources that must not be treated as arbitrary files.
- `src-tauri/src/metrics` and `src-tauri/src/power`: macOS system integration.
- `src-tauri/src/ai_usage`: provider-specific usage collection and OAuth entry
  points.
- `src/lib/utils/tauri.ts`: the only frontend IPC wrapper, including deterministic
  browser-preview mocks.
- `src/lib/stores`: Svelte state and lifecycle orchestration.
- `src/routes/dashboard` and `src/routes/quick`: the two window surfaces.
- `signatures`: reviewed cleanup definitions embedded in the Rust binary.

## Scan and cleanup flow

```text
registered signatures / domain adapters
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

See [SAFETY.md](SAFETY.md) for the full deletion contract.

## Concurrency and lifecycle

Filesystem scan and cleanup share a Rust operation mutex. This prevents the two
WebViews from starting overlapping mutations or duplicate scans against shared
state. Blocking filesystem, process, CLI, and synchronous HTTP work runs outside
the async command thread.

The quick window is persistent but inactive while hidden:

- opening reloads persistent preferences and displays cached data first;
- disk metrics refresh once per activation;
- memory polling starts only while visible and stops when hidden;
- AI usage uses a 60-second backend cache;
- scan data is reused until stale;
- Escape, Cmd+W, focus loss, and the close button hide rather than quit.

Store constructors do not start I/O. A route or an explicit activation event
owns refresh and cleanup of recurring work.

## Settings

Preferences are validated in Rust and stored at the Tauri application config
directory as `settings.json`. Writes use a temporary file followed by rename so
an interrupted save does not replace the last valid configuration. Missing
fields receive safe defaults during upgrades.

The stored settings drive theme, quick-panel section/provider order, Keep Awake
rules, excluded signatures, and Quick Clean category defaults. Launch-at-login
is deliberately marked as unavailable until a native autostart integration is
implemented.

## IPC security

Tauri capabilities are split by window:

- `capabilities/quick.json` grants read-oriented commands plus backend-owned
  safe plan creation/execution.
- `capabilities/main.json` additionally grants model deletion, Docker pruning,
  process termination, settings writes, and power controls.

The global Tauri JavaScript object is disabled and a Content Security Policy is
applied in development and production. Adding a command requires all three:
registration in `lib.rs`, declaration in `build.rs`, and an intentional window
capability entry.

## External tools

Finder-launched macOS applications often receive a smaller `PATH` than an
interactive shell. `tooling.rs` resolves CLIs through the inherited path and
common Homebrew, local-user, Docker, and Ollama locations before spawning them.
Adapters still fail closed when a required tool is unavailable.

