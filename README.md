<p align="center">
  <img src="src-tauri/icons/app-icon.svg" width="112" height="112" alt="Zenith logo" />
</p>

<h1 align="center">Zenith</h1>

<p align="center">A macOS utility for developer storage, memory, AI usage, and sleep control.</p>

Zenith helps identify reclaimable caches created by AI tools, compilers, package
managers, containers, and local model runtimes. Cleanup candidates are classified
before deletion, and credentials, configuration, source code, and user documents
remain outside the cleanup boundary.

## Features

- Storage cleanup for Claude Code, Cursor, Gemini CLI, Antigravity, Codex,
  OpenCode, Cargo, Go, Node.js, Python, Xcode, Docker, and related tools.
- Explicit `Safe`, `Rebuild`, and `Manual` cleanup tiers. Only safe items are
  selected automatically.
- Disk and local-model views with size, location, and modification details.
- Memory pressure, compression, swap, and per-application usage. Installed user
  apps can be quit normally or force quit after confirmation; system processes,
  terminals, and Zenith remain protected.
- AI usage summaries for Codex, OpenCode, and OpenRouter. Providers without an
  external usage API are clearly marked as manual.
- A configurable menu-bar panel. Storage, cleanup, AI usage, categories, and
  memory sections can be shown, hidden, and reordered.
- Native Keep Awake rules for selected applications and manual timers.
- Persistent theme, menu-bar layout, provider priority, cleanup defaults, and
  Keep Awake rules.

The menu-bar panel stops recurring metrics and provider work while hidden. Disk
metrics refresh when the panel opens, memory polling runs only while visible,
and AI usage snapshots use a short backend cache.

## Public Beta Installation (macOS)

Zenith is currently distributed as an unsigned public beta for Apple Silicon (ARM64) Macs. Pre-built `.dmg` disk images and SHA256 checksums are available under [GitHub Releases](https://github.com/jaeyoung0509/zenith/releases).

### Opening Unsigned Beta Builds on macOS

Because beta builds are not notarized with a paid Apple Developer ID, macOS Gatekeeper will display a security warning on first launch (*"cannot be opened because the developer cannot be verified"* or *"is damaged and can't be opened"*).

To launch Zenith on macOS:
1. Open the downloaded `.dmg` and drag **Zenith.app** into `/Applications`.
2. In Finder, navigate to `/Applications`, right-click (or Control-click) **Zenith.app**, and select **Open**.
3. In the confirmation dialog, click **Open**. (You only need to do this once).
4. *Alternatively*, clear the macOS quarantine attribute in Terminal:
   ```bash
   xattr -cr /Applications/Zenith.app
   ```

## Privacy & Local Diagnostics

- **100% Local**: Zero telemetry, zero cloud tracking, and zero remote analytics.
- **Secret Redaction**: Subprocess errors and diagnostic messages automatically redact sensitive API keys (`sk-...`, tokens, passwords) before writing to disk.
- **Local Logs**: Rotating error logs are stored on your Mac at `~/Library/Logs/Zenith/zenith.log`.
- **Diagnostics Export**: Inspect or export your local system snapshot anytime in **Dashboard -> Settings -> Diagnostics & Privacy Logs**.

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
- Local model weights and rebuildable caches require explicit selection.

Signature definitions live in [`signatures/`](signatures). The safety tests are
in [`src-tauri/tests/`](src-tauri/tests).

## Stack

- Tauri 2 and Rust for the desktop shell, system integration, and cleanup core
- Svelte 5, TypeScript, Vite, and Tailwind CSS for the interface
- macOS IOKit for Keep Awake assertions
- `sysinfo` and native macOS tools for memory and disk metrics

The two Tauri windows are a persistent menu-bar quick panel and the main
dashboard. Both use typed IPC commands backed by Rust modules for scanning,
cleanup, metrics, provider integrations, and power management. See
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the runtime and data flow, and
[`docs/SAFETY.md`](docs/SAFETY.md) for the deletion trust boundaries.

## Development

Requirements:

- macOS
- Rust 1.80 or newer
- Node.js 20 or newer
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
the configured application and Dock identity.

## Verification

Run the same checks expected before a change is submitted:

```bash
cargo check
cargo test
pnpm check
pnpm test -- --run
pnpm build
just build-fast
```

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
paths = [
  "~/.mytool/cache",
  "~/Library/Caches/mytool",
]
exclusions = [
  "~/.mytool/config.json",
  "~/.mytool/credentials.json",
]
description = "Compiled artifacts and temporary indices."
```

See [`AGENTS.md`](AGENTS.md) for implementation constraints and
[`DESIGN.md`](DESIGN.md) for the interface contract.

## License

MIT
