<p align="center">
  <img src="src-tauri/icons/app-icon.svg" width="112" height="112" alt="Zenith handwritten Z logo" />
</p>

<h1 align="center">Zenith</h1>

<p align="center">A cross-platform utility for developer storage, processes, local services, AI usage, and sleep control.</p>

Zenith helps identify reclaimable caches created by AI tools, compilers, package
managers, containers, and local model runtimes. Cleanup candidates are classified
before deletion, and credentials, configuration, source code, and user documents
remain outside the cleanup boundary.

## Features

- Storage cleanup for Claude Code, Cursor, Antigravity (and legacy Gemini CLI caches), Codex,
  OpenCode, Cargo, Go, Node.js, Python, Xcode, Docker, and related tools.
- An Overview control tower with CPU, memory, and disk readings, a compact
  battery status, and active tools and services. **Review resources** opens
  storage review and the protected running-app quit flow in one place.
- Explicit `Safe`, `Rebuild`, and `Manual` cleanup tiers. Only safe items are
  selected automatically.
- Optional Intensive cleanup for stale third-party application caches and logs.
  It is disabled by default and keeps Apple/system namespaces, recent data,
  settings, credentials, and user files outside the cleanup boundary.
- Developer Artifacts scans selected workspaces for generated build output and
  dependencies. Partial measurements remain explicit and cleanup requires review.
- A bounded Large Files inspector for approved user-content folders and an
  installed-application inspector with reviewed, recoverable moves to Trash.
- Disk and local-model views with size, location, and modification details.
- A Performance page with local CPU, memory and battery details. CPU is the
  system-wide share of all logical cores busy over the sampling window, and
  warm-up, stale, unavailable and failed readings are named rather than drawn as
  zero. Memory pressure leads and stays distinct from the used ratio. Battery
  shows a proportional 2D outline with the actual charge state, and the row
  disappears on a Mac without a battery.
- Memory pressure, compression, swap, and per-application usage. Installed user
  apps can be quit normally or force quit after confirmation; system processes,
  terminals, and Zenith remain protected.
- A dedicated Dev Servers tab that identifies current-user TCP listeners
  such as Vite, Next.js, agent-browser, and Chrome for Testing, shows their
  project and network exposure, and can release one exact verified listener
  without terminating unrelated browsers, Node.js, or runtime processes.
- A local Projects cockpit that groups supported AI CLI processes by canonical
  repository/worktree identity. Process-only evidence is labelled honestly,
  inaccessible contexts stay Unassigned, and PID, argv, prompts, transcripts,
  and credentials never cross into the WebView. Identity paths are masked to
  `~/…` or a basename; features whose purpose is to display a location (Storage,
  Developer Artifacts, Large Files, and App Uninstaller) are explicit exceptions
  that may show an absolute path the user selected or already owns.
- AI usage summaries for Codex, Antigravity, OpenCode, and OpenRouter. Providers without an
  external usage API are clearly marked as manual.
- AI Control Center: provenance-aware usage tracking, local budget alerts,
  advisory resource policy, and bounded project safety inspection. Provider
  availability and live, local, or manual data are identified in the interface.
- A configurable menu-bar panel. Cleanup, CPU, memory, battery, disk, storage
  categories, AI &amp; agents, and Keep Awake sections can be shown, hidden, and
  reordered; the panel keeps one fixed header and footer with a single scrolling
  region. The panel is 400 px wide and adapts its height to content, from
  300 to 740 px within the current display’s work area. Cleanup uses one compact
  status and one contextual action; detailed explanations stay in Storage.
- Native Liquid Glass on macOS 26 and later, with vibrancy on older macOS.
  Light, Dark, and System preferences also control the native material. Reduced
  transparency uses opaque chrome; Windows keeps its own opaque adapter.
- Reviewed local brand identities for the tools Zenith names, with the source,
  licence and notices recorded in `docs/design/brand-assets.md`. Everything
  unresolved falls back to a neutral glyph beside the factual product name, and
  no icon is fetched at runtime.
- Native Keep Awake rules for selected applications and manual timers.
- Persistent theme, menu-bar layout, provider priority, cleanup defaults, and
  Keep Awake rules.

The menu-bar panel stops recurring metrics and provider work while hidden. Disk
metrics refresh when the panel opens, memory polling runs only while visible,
and AI usage snapshots use a short backend cache.

## Public Beta Installation

Zenith is distributed as a public beta for Apple Silicon (ARM64) Macs and
Windows x64. Pre-built `.dmg` and NSIS `.exe` installers, an SPDX software bill
of materials, SHA256 checksums, build metadata, a recorded endpoint-protection
review, and GitHub build provenance attestation are available under
[GitHub Releases](https://github.com/jaeyoung0509/zenith/releases).

### Windows x64

Two NSIS installers are published for Windows x64:

- `Zenith-windows-x64-setup.exe` installs for the current user under
  `%LOCALAPPDATA%\Zenith` and requires no administrator access.
- `Zenith-windows-x64-setup-machine.exe` installs for all users under
  `Program Files\Zenith` and requires elevation. It exists for managed machines
  whose application-control policy (AppLocker default rules, WDAC, or Smart App
  Control) refuses to execute binaries from user-writable locations. Installing
  into `Program Files` does not make the binary trusted.

Both installers embed Microsoft's WebView2 offline installer. Installation
therefore completes without network access and the download grows by roughly
130 MB. When WebView2 is missing, the installer runs Microsoft's WebView2
installer with its own interface, so a failure is visible during installation
instead of surfacing later as a window that never opens.

The Windows beta is intentionally unsigned while Zenith completes the
[SignPath Foundation](https://signpath.org/) open-source onboarding process.
Microsoft Defender SmartScreen will identify it as an unknown publisher, and
because SignPath Foundation issues organization-validated certificates that
carry no SmartScreen reputation, **that warning is expected to persist after
the first signed release**. Confirm that the installer came from this
repository's GitHub Release and verify its SHA256 value against
`SHA256SUMS.txt` before choosing **More info → Run anyway**. See the
[Windows guide](docs/WINDOWS.md) and [code signing policy](CODE_SIGNING_POLICY.md)
for the install modes, the endpoint-protection review gate, and the GitHub
build provenance attestation.

### macOS ARM64

#### Local release recipes

- `just distribute` only creates fresh `.app` and `.dmg` package artifacts under `target/release`; it never changes `/Applications`.
- `just release` builds only the `.app`, validates its bundle identity and version, then replaces the exact `/Applications/Zenith.app`. It does not create a DMG or open Finder. The previous installed bundle is restored if activation or verification fails.
- `just release-and-run` performs the same verified replacement and opens the installed copy rather than the build-tree bundle.
- `just install-release` installs an already-built release bundle using the same transaction. It reports a clear error if the current user cannot write to `/Applications` and does not use `sudo` automatically.

If a Tauri release build fails, the command prints the path of a retained local
build log. Keep that log and the mounted-image state before retrying a DMG
build; redact user paths before sharing either. A successful build removes its
temporary log.

#### Opening unsigned beta builds on macOS

The macOS beta is unsigned and not notarized. That is a dated decision recorded
in the [code signing policy](CODE_SIGNING_POLICY.md#macos-notarization): Zenith
does not hold a paid Apple Developer ID, so the `.dmg` is never submitted to
Apple's notary service and carries no stapled ticket. macOS Gatekeeper therefore
blocks the first launch and displays a security warning (*"cannot be opened
because the developer cannot be verified"* or, for some download paths,
*"is damaged and can't be opened"*).

To launch Zenith on macOS:
1. Open the downloaded `.dmg` and drag **Zenith.app** into `/Applications`.
2. In Finder, navigate to `/Applications`, right-click (or Control-click) **Zenith.app**, and select **Open**.
3. In the confirmation dialog, click **Open**. (You only need to do this once).
4. *Alternatively*, clear the macOS quarantine attribute for this bundle in Terminal:
   ```bash
   xattr -cr /Applications/Zenith.app
   ```
   This removes the malware check for that one bundle, so verify the published
   SHA256 checksum and the GitHub build provenance attestation first.

Do not disable Gatekeeper system-wide. Verify the DMG against `SHA256SUMS.txt`
before overriding any warning.

### Knowing when a corrected version exists

Zenith has no automatic updater and does not check for new releases in the
background. Enabled AI integrations can still contact their providers for usage
data; this is separate from update checking. To find
out, open the [GitHub Releases](https://github.com/jaeyoung0509/zenith/releases)
page and compare the newest tag with the version shown in Zenith. The
application exposes that release URL (`PlatformContext.releases_url`) and links
to it from the interface, so the check is one click away, but every check is
user-initiated.

## Privacy & Local Diagnostics

- **Local processing**: Scans, cleanup decisions, and diagnostics stay on your
  machine. Zenith sends no analytics or telemetry. Enabled AI integrations may
  contact their provider APIs or official tools for authentication and usage.
- **Secret Redaction**: Subprocess errors and diagnostic messages automatically redact sensitive API keys (`sk-...`, tokens, passwords) before writing to disk.
- **Local Logs**: Error logs are written under your own platform's application-data directory -- `~/Library/Logs/Zenith/zenith.log` on macOS, `%LOCALAPPDATA%\Zenith\Logs\zenith.log` on Windows -- and rotate to `zenith.log.1` beside them once the live log exceeds 1 MB. The Settings diagnostics view shows the path this machine actually resolved, and Windows layout details are in [docs/WINDOWS.md](docs/WINDOWS.md).
- **Diagnostics Export**: Inspect or export your local system snapshot anytime in **Dashboard -> Settings -> Diagnostics & Privacy Logs**.
- **Doctor Self-Check**: run `Zenith --doctor` (or `Zenith --doctor --json`) to print a de-identified environment fingerprint and a self-check table; the command exits 1 when a self-check fails. It performs no network access, and nothing it prints contains a user name, machine name, drive letter, or profile path. Windows bug reports ask for this output because it is safe to paste.
- **Minimized Agent Metadata**: Project Cockpit returns opaque project/session
  IDs, compact location hints, resource totals, and evidence labels. It does not
  return process IDs, command lines, environment values, prompts, or transcripts.
- **AI Control Privacy**: AI Control Center reuses those opaque identities for
  provenance-aware usage, advisory resource policy, bounded safety inspection,
  and post-baseline Git metadata. See [AI Control Center](docs/AI_CONTROL_CENTER.md).

## Cleanup safety

Zenith does not expose an arbitrary path deletion command. Every cleanup target
must come from a registered signature and pass the safety planner before it can
be executed.

- System paths, home roots, credentials, keychains, source repositories, and
  standard user-content folders are blocked.
- Symlinks are not followed.
- Planned files are checked again immediately before deletion using filesystem
  identity metadata to reduce time-of-check/time-of-use risk.
- Temporary-file cleanup is restricted to known tool prefixes and inactivity
  thresholds; Zenith never scans or deletes all of `/tmp`.
- OrbStack's reviewed VM disk is reported as manually managed container storage
  using allocated bytes; Zenith never deletes or compacts it through generic
  cleanup.
- Intensive cleanup considers only stale direct children of approved user cache
  and log roots. Symlinks and protected Apple/system namespaces are skipped,
  and inactivity is checked again immediately before deletion.
- Local model weights and rebuildable caches require explicit selection.
- Large Files and App Uninstaller retain backend-owned inventories and use
  short-lived, one-shot Trash plans. The WebView submits opaque IDs rather than
  filesystem paths or deletion strategies.

## Process and development-port safety

Zenith never exposes an arbitrary PID-kill command. Application Quit actions
resolve a fresh allowlisted app group in Rust, while Development Servers uses a
separate endpoint-level workflow:

- Only current-user TCP listeners on non-privileged ports are considered.
- Runtime names such as `node` or `python` are insufficient by themselves; a
  conservative development-server signature and stable process identity are
  required.
- Testing-tool listeners require exact official executable paths. Chrome for
  Testing additionally requires remote-debugging and isolated-profile arguments;
  standard Google Chrome and renamed lookalike binaries remain ineligible.
- The UI receives a short-lived opaque listener ID, not termination authority
  over a PID, path, process group, or signal.
- Immediately before signaling, Rust rechecks the PID, port, bind address, UID,
  process start time, executable identity, classification, and protected rules.
- Normal release sends `SIGTERM` to the one verified listener. `SIGKILL` is
  available only after that listener remains alive and the backend issues a new
  force-authorized one-shot ID for a second confirmation.
- PID reuse, port handoff, expired IDs, missing identity data, system services,
  terminals, databases, container daemons, and Zenith itself fail closed.

Signature definitions live in [`signatures/`](signatures). Domain safety tests live in [`crates/zenith-core`](crates/zenith-core); desktop
adapter and integration tests also live in [`src-tauri/tests/`](src-tauri/tests).

## Stack

- Tauri 2 and Rust for the desktop shell, system integration, and cleanup core
- Svelte 5, TypeScript, Vite, and Tailwind CSS for the interface
- macOS IOKit for Keep Awake assertions
- `sysinfo` and bounded native macOS tooling for memory, disk, process, and TCP
  listener inspection
- Rayon directory-level work stealing with a hardware-aware worker cap for
  signature-scoped size measurement

The two Tauri windows are a persistent menu-bar quick panel and the main
dashboard. Both use typed IPC commands backed by Rust modules for scanning,
cleanup, metrics, provider integrations, and power management. See
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the runtime and data flow,
[`docs/SAFETY.md`](docs/SAFETY.md) for the deletion trust boundaries, and
[`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md) for the credential and privacy
boundaries plus the platform ceilings that cannot be removed.
Windows contributors can follow [`docs/WINDOWS.md`](docs/WINDOWS.md) for the
MSVC, Tauri, and NSIS development workflow.

## License

Zenith is available under the [MIT License](LICENSE). Official Windows release
signing follows the project's [code signing policy](CODE_SIGNING_POLICY.md).

## Development

Requirements:

- macOS, or Windows 10 1809 or newer with the MSVC build tools (see
  [docs/WINDOWS.md](docs/WINDOWS.md))
- Rust 1.95 or newer. The workspace declares `rust-version = "1.95.0"` in
  `Cargo.toml`, and CI verifies the build against exactly that toolchain.
- Node.js 22.22.2 or newer on the 22.x line (24.15+ or 26+ also work);
  `package.json` declares this floor, which comes from the frontend
  toolchain, and CI runs the typecheck and test gate on exactly that
  version.
- pnpm
- `just` (recommended)

Install dependencies and run the desktop development build:

```bash
pnpm install
just dev
```

For browser-only interface work with mocked Tauri commands:

```bash
just dev-web
```

Build and open a debug `.app` bundle:

```bash
just build-fast
just run-fast
```

The app bundle should be used for local macOS verification because it preserves
the configured application and Dock identity. `just build-fast` does not replace
the installed `/Applications/Zenith.app`; use the release recipes above when you
intend to update that copy.

To increment the patch version and verify all manifests agree:

```bash
just bump-patch
just check-version
```

Rebuild after a version change so the running app includes the updated UI and
version. An already-running process continues to use its previous assets.

## Verification

Run the same checks expected before a change is submitted:

```bash
just lint              # cargo fmt --check, cargo clippy -D warnings, pnpm check
just test              # Rust tests, Vitest, release-installer regression
just check             # Rust lint, architecture, cargo check, pnpm check/build
just supply-chain      # cargo deny, cargo audit, pnpm audit
just build-fast        # macOS: debug .app bundle
```

### What CI does and does not verify

CI verifies frontend typecheck, Vitest, binding drift, and the Vite build; Rust
formatting, clippy, unit and safety tests on macOS and Windows x64; packaging
smoke builds for macOS and Windows; the declared MSRV build with Rust 1.95.0;
and locked-dependency audits for both ecosystems. On Windows it also silently
installs the per-user installer, runs `Zenith --doctor`, requires every
self-check to pass, silently uninstalls, and fails if the install directory
survives, and the machine-wide installer runs the same gate with its install scope asserted. The same job builds both installers.

CI does **not** verify redirected user folders, machines whose system drive is
not `C:`, UNC or domain-joined profiles, non-NTFS volumes, non-UTF-8 code pages,
standard-user (unprivileged) installation, application-control policy
configurations, or machines without the WebView2 runtime under interactive use.
The Windows validation matrix in
[docs/WINDOWS_VALIDATION.md](docs/WINDOWS_VALIDATION.md) is a plan for manual
runs; a row is evidence only once a run records its application version,
Windows build, and date.

## Adding a cleanup signature

Add or update a TOML file under [`signatures/`](signatures). Keep paths narrowly
scoped and exclude configuration, credentials, and user-owned state.

```toml
[[signatures]]
id = "dev.mytool.cache"
name = "MyTool Cache"
category = "developer"
risk = "safe" # safe | rebuild | manual
strategy = "delete_contents" # delete_contents | delete_directory
paths = ["~/.mytool/cache"]
exclusions = [
  "~/.mytool/config.json",
  "~/.mytool/credentials.json",
]
description = "Compiled artifacts and temporary indices."
```

Signatures used only by the opt-in broader scan must declare
`intensive_only = true`. Broad roots must also declare a minimum age and prefix
protections so the scanner emits reviewable direct children instead of the root
itself:

```toml
[[signatures]]
id = "system.intensive.example_cache"
name = "Stale Example Cache"
category = "system"
risk = "safe"
strategy = "delete_stale_contents"
platforms = ["macos"]
paths = ["~/Library/Caches"]
min_age_days = 7
exclude_prefixes = ["com.apple."]
intensive_only = true
description = "Third-party cache trees inactive for at least seven days."
```

A root may be a selector when the catalog cannot spell it out: `*` matches one
directory name and `{a,b}` matches one of several, so one entry covers every
browser profile or every Electron cache subtree. A selector belongs in `paths`
only — an exclusion that contained `*` would protect nothing — and the
signature's `unit` decides whether each match is the cleanup unit
(`named_subtree`) or a root whose children are (`child_namespace`). Selector
expansion never follows a link, is capped at 256 matches per pattern, and is
reported as truncated when it reaches the cap. Windows spellings are accepted
too: `%LOCALAPPDATA%\Temp` is normalized to the placeholder the expander
resolves, and an environment variable this build does not know fails the load
instead of resolving to nothing.

A signature states how its age policy applies. `delete_directory` ages a child
as a whole and removes all of it or none. `delete_stale_contents` ages the
entries inside a unit and removes only those (`min_age_days` is required with
it), which is what a cache namespace needs when an application writes to it
while the cleaner works: a file touched this morning no longer hides the
gigabytes beside it. Structured state — databases, locks, credentials,
configuration, bundles, executables — is never removed by either policy, in a
unit root or inside a pruned tree.

A signature can also state what its deletion would disturb. `fail_if_running`
names the executables whose running state refuses the cleanup (a compiler or
runtime holding its own cache open). `owner` names who the catalog expects to
own the location, and defaults to `provider` when it is omitted. `discovery`
defaults to `mode_gated`; a signature that declares `always` is inventoried in
either scope, and the scope then decides only eligibility — the units it finds
in standard mode are reported as discovered and not cleanable. `priority`
decides which of two signatures that describe the same location wins, so a scan
is deterministic rather than dependent on map order.

See [`AGENTS.md`](AGENTS.md) for implementation constraints and
[`DESIGN.md`](DESIGN.md) for the interface contract.
