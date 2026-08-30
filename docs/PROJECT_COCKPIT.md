# Project Cockpit & AI Agent Activity Center

The Project Cockpit is Zenith's developer-first activity center that unites active
AI agent CLIs, running development services, and developer storage into a unified,
canonical project view.

## Core Features & Two-Level UX

The Level 1 view is organized into three top-level sub-tabs directly below the
page header. `Usage` is the default entry point; `Projects` contains the
verified workspace/session view; and `Tool Adapters` contains the supported
adapter matrix. The outer sidebar route remains `projects` for compatibility.

Each sub-tab owns its loading and error boundary. Usage loads the cached quota
snapshot on first activation, Projects loads the local agent snapshot on first
activation, and Tool Adapters reuses that snapshot while loading integration
metadata only when opened. Returning to a visited tab reuses its successful
data. The header refresh action is scoped to the active tab (`Refresh usage`,
`Refresh projects`, or `Refresh tool adapters`), and a failed refresh never
removes another domain's last successful data.

The sub-tabs use the ARIA tablist pattern. Arrow keys move between adjacent
tabs, while Home and End move to the first and last tab. Selecting a project
opens the existing Level 2 cockpit in the Projects context; `Back to Projects`
returns to the Projects list.

### Level 1: Project List (`Projects` sub-tab)
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

### Level 1: Usage (`Usage` sub-tab)

- **Connected AI Accounts & Quota**: Shows only provider usage cards and the
  connected-provider count. Provider provenance remains explicit (live, local,
  or manual), and OAuth token files never reach the UI.
- **Scoped refresh**: Refreshing Usage calls only the usage store and preserves
  the last successful cards beside an inline error when a refresh fails.

### Level 1: Tool Adapters (`Tool Adapters` sub-tab)

- **Adapter matrix**: Shows the eight supported local process adapters and
  their evidence/state messages. Project cards and account cards are not
  duplicated here.
- **Lazy integration metadata**: Integration status is fetched on first entry
  to this sub-tab, with legacy-marker removal retaining its existing safe store
  path. Process-only observations never claim detailed agent state.

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
| **Antigravity** | `agy`, `antigravity` | Process-only | `Process observed` | No verified bridge |
| **Claude Code** | `claude` | Process-only | `Process observed` | No verified bridge |
| **Cursor Agent CLI** | `cursor-agent` | Process-only | `Process observed` | No verified bridge |
| **Grok Build** | `grok` | Process-only | `Process observed` | No verified bridge |
| **GitHub Copilot CLI** | `copilot` | Process-only | `Process observed` | No verified bridge |
| **Gemini CLI (Legacy)** | `gemini` | Process-only | `Process observed` | N/A (transitioned to Antigravity) |
| **Codex CLI** | `codex` | Process-only | `Process observed` | N/A |
| **OpenCode** | `opencode` | Process-only | `Process observed` | N/A |

## Truthful Status & Evidence Model

Zenith strictly distinguishes between vendor-confirmed events and ambient OS process observation:
- **Vendor confirmed / Vendor event**: Reserved for a validated local lifecycle event
  that identifies exactly one observed process. No bundled adapter currently emits this evidence.
- **Process observed**: The exact allowlisted CLI is running under the current user's UID.
  Detailed internal state is labeled as `Process observed · detailed status unavailable`
  rather than guessing.
- **Possibly inactive**: A process repeatedly observed without measurable CPU activity
  for the configured inactivity threshold (default 15 minutes). Process age alone is never
  treated as inactivity.
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
- **Configurable Events**: Repeatedly observed inactivity alerts are active. Turn-complete
  and approval/input alerts remain dormant until a verified vendor event bridge is available.
- **Privacy Masking**: Full paths, branch names, prompts, transcripts, and credentials are
  strictly omitted. The "Hide project name" option replaces folder names with "an active project".
- **Deduplication**: Filtered by `(session_id, event_kind, turn_id)` to prevent spam.

## Menu Bar Quick Panel Section

- Displays active session counts and attention-needed indicators.
- Up to 3 recent session rows with project name, tool name, and duration.
- Deep link button to "Open Projects" in the main window.
- **Zero background overhead**: No polling or CLI execution occurs when the quick panel is hidden.
- **Read-only**: Stop actions are strictly prohibited in the quick panel capability.
