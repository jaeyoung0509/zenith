# Project Cockpit

Project Cockpit is a local, read-only view of supported AI CLI processes grouped
by verified project or Git worktree context. It does not launch, steer, stop, or
configure third-party agents.

## Supported process observation

The initial adapter registry recognizes exact executable identities for:

- Antigravity (`agy`) and legacy/enterprise Gemini CLI (`gemini`)
- Codex CLI (`codex`) and Claude Code (`claude`)
- Cursor Agent CLI (`cursor-agent`), not the Cursor GUI process
- Grok Build (`grok`), GitHub Copilot CLI (`copilot`), and OpenCode (`opencode`)

Existing processes are reported as **Process observed**. This evidence proves
that the exact CLI is running, but not that a vendor task is waiting, finished,
or stalled. Adapters with a documented local integration are labelled
**Integration available**; Zenith does not install or modify those integrations
in this phase.

## Data collected

Rust inspects the current user's process executable identity, start time, cwd,
CPU, and resident memory. It uses cwd only to establish a canonical folder,
repository, or linked-worktree identity. The frontend receives:

- opaque project, repository, and session IDs;
- project display name, compact parent/name hint, worktree flag, and branch;
- tool name, observation evidence, elapsed time, CPU, and memory;
- adapter capability/health and sanitized partial errors.

Zenith does **not** return or persist PID, full paths, argv, environment values,
prompts, responses, tool input/output, transcript data, email addresses,
credentials, Git remotes, changed file names, or diffs. No Zenith cloud service
or telemetry endpoint is involved.

## Failure and troubleshooting

- **No active sessions:** start a supported CLI from a project and refresh.
- **Unassigned session:** the executable was verified, but cwd was missing,
  inaccessible, or could not be canonicalized. Zenith does not guess a project.
- **Process-only:** the running tool does not expose verified lifecycle detail to
  this phase, so only active process observation is shown.
- **Not observed:** no exact current-user executable identity was present in the
  snapshot. Renamed wrappers and substring matches are intentionally ignored.
- **Refresh failed:** the previous successful snapshot remains visible. Retry
  after checking local process/filesystem permissions.

The Projects tab can be hidden or reordered under Settings. This does not alter
any third-party tool configuration, and there is nothing to uninstall beyond
disabling the tab.
