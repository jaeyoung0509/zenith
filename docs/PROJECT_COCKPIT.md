# Project Cockpit & AI Agent Activity Center

The Project Cockpit is Zenith's developer-first activity center that unites active
AI agent CLIs, running development services, and developer storage into a unified,
canonical project view.

## Core Features & Two-Level UX

### Level 1: Project List
- **Connected AI Accounts & Quota**: Glanceable horizontal strip displaying connected AI provider status, weekly quota usage percentages, and reset countdowns (Codex, Claude, OpenRouter, Antigravity).
- **Canonical Projects**: Groups running agent processes, development listeners, and
  project storage by verified Git repository or worktree root.
- **Git State**: Displays branch name or detached HEAD indicator, worktree badge, and
  dirty working tree status without modifying repository state.
- **Attention Counters**: Surfaces sessions needing user input, tool approval, or turn
  completion review with prominent attention badges.
- **Correlated Quick Chips**: One-click access to correlated dev server ports (e.g.
  `:5173`) and developer artifact storage sizes (e.g. `50 MB`).
- **Unassigned Sessions Card**: Clearly presents verified agent processes whose working
  directory could not be correlated to a known project root without guessing.
- **Tool Adapters & Local Integrations**: Displays the truthful status and version of all
  8 supported AI tool adapters with one-click install/remove actions.

### Level 2: Project Cockpit
- **Repository Actions**: Open in Terminal and Reveal in Finder controls.
- **Active Agent Sessions**: Truthful lifecycle status, evidence classification, elapsed
  runtime, CPU %, memory usage, and graceful stop controls for eligible sessions.
- **Correlated Development Services**: List of running ports with deep links to the
  Development Servers tab.
- **Correlated Developer Storage**: Total node_modules, build cache, and artifact sizes
  with deep links to Developer Storage.

## Supported Tool Adapters Matrix

| Tool | Executable Names | Integration Mode | Evidence Reported | Hook Config Path |
| :--- | :--- | :--- | :--- | :--- |
| **Antigravity** | `agy`, `antigravity` | Local status hook / CLI | `Vendor confirmed`, `Process observed` | `~/.gemini/antigravity/hooks.json` |
| **Claude Code** | `claude` | Official settings hook | `Vendor confirmed`, `Process observed` | `~/.claude/settings.json` |
| **Cursor Agent CLI** | `cursor-agent` | Local lifecycle hook | `Vendor confirmed`, `Process observed` | `~/.cursor/hooks.json` |
| **Grok Build** | `grok` | Local lifecycle hook | `Vendor confirmed`, `Process observed` | `~/.grok/hooks.json` |
| **GitHub Copilot CLI** | `copilot` | Local lifecycle hook | `Vendor confirmed`, `Process observed` | `~/.copilot/hooks.json` |
| **Gemini CLI (Legacy)** | `gemini` | Process-only | `Process observed` | N/A (transitioned to Antigravity) |
| **Codex CLI** | `codex` | Process-only | `Process observed` | N/A |
| **OpenCode** | `opencode` | Process-only | `Process observed` | N/A |

## Truthful Status & Evidence Model

Zenith strictly distinguishes between vendor-confirmed events and ambient OS process observation:
- **Vendor confirmed / Vendor event**: Backed by a verified local hook lifecycle event
  or status line. Shows precise states (`Working`, `Waiting for User`, `Starting`, `Idle`, `Turn Complete`).
- **Process observed**: The exact allowlisted CLI is running under the current user's UID.
  Detailed internal state is labeled as `Process observed · detailed status unavailable`
  rather than guessing.
- **Possibly inactive**: An observed process with zero CPU usage for longer than the
  configured inactivity threshold (default 15 minutes).
- **Exited**: A previously observed session that terminated, retained in memory for 60
  seconds with its exit timestamp before eviction.

## Correlation Engine

- **Deepest Canonical Ancestry Matching**: Agent working directories and dev listener
  paths are resolved via `canonicalize()` to prevent false positives with symlinks,
  same-basename folders, or monorepo subdirectories.
- **Worktree Independence**: Linked Git worktrees have distinct `worktree_id` and
  `ProjectIdentity` values and are never merged into their main repository.
- **Unassigned Fallback**: Sessions without an accessible or provable directory remain in
  `unassigned_sessions`. Zenith never guesses correlation.

## Graceful Stop Architecture

- **Opaque Leases**: The backend generates short-lived (30s) opaque tokens (`StopLease`).
  The frontend submits only `sessionId` and `leaseId`; it never submits PIDs or signals.
- **TOCTOU & PID Reuse Protection**: Before signaling, Rust verifies:
  1. Process exists and is owned by current user UID.
  2. Process `start_time` exactly matches the recorded lease start time (preventing PID reuse).
  3. Executable path exactly matches the allowlisted agent adapter binary.
  4. Process is not a protected terminal emulator, shell, login, or system process.
- **Signal Policy**: Sends `SIGTERM` only. Never `SIGKILL`, never sends signals to process
  groups or parent shells.

## Desktop Notifications & Privacy

- **Opt-In**: Notifications are disabled by default.
- **Configurable Events**: Turn completed, approval/input needed, and inactivity alerts.
- **Privacy Masking**: Full paths, branch names, prompts, transcripts, and credentials are
  strictly omitted. The "Hide project name" option replaces folder names with "an active project".
- **Deduplication**: Filtered by `(session_id, event_kind, turn_id)` to prevent spam.

## Menu Bar Quick Panel Section

- Displays active session counts and attention-needed indicators.
- Up to 3 recent session rows with project name, tool name, and duration.
- Deep link button to "Open Projects" in the main window.
- **Zero background overhead**: No polling or CLI execution occurs when the quick panel is hidden.
- **Read-only**: Stop actions are strictly prohibited in the quick panel capability.
