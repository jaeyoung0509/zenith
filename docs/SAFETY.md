# Cleanup Safety Model

Zenith deletes developer caches, so a correct UI is not considered a security
boundary. Every destructive decision is reconstructed and validated in Rust.

## Trust boundaries

The frontend may provide:

- a current `scan_id`;
- selected opaque item IDs;
- an opaque one-shot `plan_id` after reviewing a preview;
- typed identities for dedicated adapters, such as a freshly scanned model ID;
- a Large Files request containing only supported root tokens and a size
  threshold plus a backend-validated file filter;
- an app inventory ID or app-inspection ID returned by the backend.

The frontend may not provide an executable deletion path, cleanup strategy,
risk tier, filesystem identity, arbitrary PID, arbitrary Large Files root, or
arbitrary app-leftover path. These values are resolved from current backend
state.

## Generic filesystem cleanup

A generic target is executable only when all of the following hold:

1. It belongs to the backend's current scan.
2. Its signature exists in the embedded registry.
3. Its resolved path remains inside the signature scope.
4. It is not a `Manual` target.
5. The plan is unexpired and has not been used before.
6. Its filesystem identity still matches immediately before deletion.
7. Every traversed entry passes blacklist and signature-exclusion checks.

The tree deleter walks bottom-up without following symlinks. It unlinks a
symlink itself, preserves blacklisted or excluded descendants, and removes a
directory only after it is empty. Generic cleanup does not call
`remove_dir_all`.

Blocked locations include filesystem roots, the user home root, credentials,
keychains, source-control metadata such as nested `.git`, and standard user
content directories. Temporary cleanup never targets all of `/tmp`; candidates
must match known tool prefixes and inactivity rules.

OrbStack storage is observation-only. Zenith reads allocated block metadata
from the single reviewed `data.img.raw` path so users can account for managed
container storage, but it does not scan arbitrary group containers or create a
generic cleanup target for the VM disk. Manual adapter observations are rejected
by the planner even if a frontend attempts to select one.

## Intensive cleanup

Intensive cleanup broadens discovery without weakening deletion authority. It
is disabled by default, persisted as a validated setting, and only enables
registered signatures marked `intensive_only`.

Broad user cache and log signatures are constrained as follows:

- only direct children of the registered root can become targets;
- the root itself is never returned as a cleanup item;
- symlink children and protected Apple/system prefixes are skipped;
- the newest timestamp anywhere in the candidate tree must exceed the declared
  minimum inactivity age;
- incomplete traversal, permission failure, or recursion depth cutoff excludes
  the candidate;
- the planner accepts only a direct child of the resolved signature root; and
- the executor repeats the full-tree inactivity check immediately before
  deletion and aborts if anything became recent.

The current thresholds are seven days for third-party children under
`~/Library/Caches` and fourteen days for application-log groups under
`~/Library/Logs`. Diagnostic and crash-report groups remain protected.
Intensive mode does not scan user documents, preferences, credentials,
databases, browser profiles, model weights, arbitrary system cache roots, or
unknown `/tmp` children.

## Large Files Inspector

Large Files is intentionally not generic cleanup. User content is protected by
the generic blacklist because automated cleanup must never decide that a
Document, Desktop file, or Movie is disposable. Large Files instead means “show
space usage and let the user explicitly choose.”

The backend accepts only these root tokens:

- `downloads`
- `desktop`
- `documents`
- `movies`

Each token is resolved against the current user home directory in Rust. The
frontend cannot submit `/`, `~/Library`, an external volume, or another arbitrary
path.

Traversal and execution enforce these rules:

1. Symlinked roots and candidates are rejected, symlinks are never followed,
   and every path component is checked again before Trash.
2. A traversal does not cross the device ID of the selected root.
3. `.app`, Photos-library, Music-library, and iMovie-library packages are not
   descended as ordinary files.
4. Paths containing `.git` remain protected even though user-content roots are
   intentionally allowed for this workflow.
5. Results are bounded to the 10,000 largest matches. If more files match, the
   result is marked as truncated and the UI discloses the limit.
6. Selection is empty by default. There is no Quick Clean integration.
7. A Trash plan resolves only opaque IDs from the current backend inventory.
8. Immediately before Trash, the executor verifies the item still lives inside
   an approved Large Files root, still has the reviewed parent, is still a file,
   is not a symlink, and has the same filesystem identity.

The optional `installers` filter is narrower than the normal Large Files scan:
it accepts only `.dmg`, `.pkg`, `.mpkg`, `.xip`, and `.iso` files and lowers the
minimum size to 10 MB. The default `all` filter retains the 100 MB floor. The
filter is resolved in Rust, installer results are never auto-selected, and the
frontend cannot provide an arbitrary extension or root. Both modes use the same
bounded inventory and native Trash plan; moving an installer to Trash is not
reported as reclaimed space until the user empties Trash.

The specialized Large Files scope does not change `Blacklist` behavior for any
other cleaner. In particular, `Documents`, `Desktop`, and `Movies` remain
blacklisted for generic signature-based cleanup.

## Developer Artifact Review

Developer Artifact Review is manual inventory, not Quick Clean. `Scan this
computer` registers the canonical current-user home as a backend-owned scan scope;
the frontend receives only its opaque workspace ID and cannot submit a path,
scope, or cleanup rule. The native picker remains available for narrower
user-owned folders beneath home.

The whole-home scope prunes protected paths before traversal. It bypasses
`Library`, credential stores such as `.ssh`, `.gnupg`, `.aws`, `.azure`, and
`.kube`, user media/content roots protected by the global blacklist, installed
`.app` bundles, and known top-level package-manager/runtime state directories.
Symlinks are not followed. These bypassed paths do not become candidates and
do not consume recursive measurement work.

Discovery uses reviewed ecosystem evidence before a directory becomes a
candidate. Project markers must be direct children of the exact project root;
an ancestor marker never authorizes a same-named directory deeper in the
source tree:

- `target` requires `Cargo.toml` or `pom.xml`;
- `node_modules` requires `package.json`;
- `.venv`/`venv` requires Python dependency metadata and `pyvenv.cfg`;
- `build`/`.gradle` requires Gradle markers; CMake `build` additionally
  requires its generated `CMakeCache.txt`;
- Composer `vendor` requires Composer metadata generated inside `vendor`, and
  `vendor/bundle` requires Bundler markers;
- `bin`/`obj` requires a direct .NET project marker;
- `.build`, `.dart_tool`, `_build`, `deps`, and `.terraform` require Swift,
  Dart, Elixir, or Terraform markers respectively; and
- `~/go/pkg/mod` is shown as a separate shared cache when the user selects the
  `go` workspace root or uses the backend-owned whole-home scope.

Unknown `build`, `dist`, `out`, `cache`, `vendor`, or hidden directories are
never executable based on their names alone. Discovery skips `.git`, symlinks,
other filesystems, and recognized artifact trees. Candidate measurement is a
single bounded traversal that records logical/allocated bytes, file count, and
newest modification time. A permission error, symlink, depth cutoff, or marker
change marks the candidate incomplete and blocks planning. Measurement workers
are bounded at four and cancellation stops new work while retaining only
completed candidates.

Age is informational. Recent artifacts remain selectable when the user chooses
them, and old artifacts are not selected automatically. Before a Trash plan is
created, Rust resolves selected IDs from the fresh inventory and rejects
incomplete records. Immediately before each move, it revalidates the workspace
and project identities, exact relative artifact type, marker identities,
symlink-free scope, directory type, and candidate identity. Project roots,
workspace roots, `.git`, source paths, forged IDs, stale inventories, replayed
plans, and frontend-provided paths fail closed. Selecting a project directory
as the workspace remains valid because only its exact generated child (for
example, `target/`) enters the plan; the project directory itself never does.

## App Uninstaller

App Uninstaller also uses its own backend-owned inventory and Trash plan. It does
not treat app leftovers as generic cache signatures.

Application inventory is limited to direct `.app` children of `/Applications`
and `~/Applications`. System applications are outside the removable inventory.
The selected app is resolved by opaque ID and its filesystem identity is stored
in Rust. A running app is rejected before uninstall inspection, and Zenith
cannot create an uninstall inspection for itself. The backend retains only the
current inspection, so selecting another app invalidates the earlier review.

Related-data discovery is precision-first. The current approved roots are:

- `~/Library/Application Support`
- `~/Library/Caches`
- `~/Library/Logs`
- `~/Library/Preferences`
- `~/Library/Saved Application State`
- `~/Library/Containers`
- `~/Library/Group Containers`
- `~/Library/Application Scripts`
- `~/Library/HTTPStorages`
- `~/Library/WebKit`

Exact bundle-identifier matches are high confidence. Exact display-name matches
are medium confidence and are not selected by default. Group Containers are
classified as shared rather than high confidence unless stronger ownership
evidence is available. Substring/fuzzy matches are rejected.

The app bundle is a deliberate exception to the generic `/Applications`
blacklist, but only inside this dedicated workflow. Immediately before Trash,
the executor requires it to still be a direct child of `/Applications` or
`~/Applications`, still end in `.app`, and still match the reviewed filesystem
identity. It also checks for a newly running app and symlinks in every path
component. Related Library items continue to pass the generic blacklist in
addition to the approved Library-root check. If the app bundle cannot be moved,
the executor skips all related data rather than performing a leftovers-only
partial uninstall.

App bundle identity intentionally includes directory metadata in addition to
device and inode. Any observed bundle change after review is treated as stale
and requires a fresh inspection rather than weakening the fail-closed check.

## Native Trash semantics

Large Files and App Uninstaller move reviewed targets to the platform's native
Trash or Recycle Bin through the dedicated Trash adapter. They do not call the
generic tree deleter, `rm`, or `remove_dir_all`.

Trash plans expire after five minutes and are removed from the backend plan map
before execution, making them one-shot. Every target is revalidated immediately
before its individual Trash operation. Partial failures are reported per item.

Moving an item to Trash does not guarantee that disk space has already been
freed. Product copy must say “Moved to Trash” and may describe the moved amount
as potentially reclaimable after the Trash is emptied.

## Risk tiers

- `Safe`: disposable cache or log data; may be selected by default.
- `Rebuild`: recoverable through download or recompilation; opt-in unless the
  user explicitly enables rebuild items for Quick Clean.
- `Manual`: stateful or ambiguous data; never executable by Generic Cleaner.

Manual resources use dedicated adapters with their own identity and confirmation
rules. Docker volumes require a warning confirmation. Local models are resolved
from a fresh inventory; Ollama uses the official CLI identity rather than a
manifest filesystem path.

## Process termination

Project Cockpit is observation-only. Its adapter registry matches exact CLI
executable identities for Antigravity (`agy`), legacy/enterprise Gemini CLI,
Codex, Claude Code, Cursor Agent CLI, Grok Build, Copilot CLI, and OpenCode.
Names and substrings alone are insufficient, and Cursor's GUI process never
creates an agent session. Current-user ownership, a non-zero start time, an
executable path, and cwd are checked before project correlation. No termination
lease or mutable command is exposed by this workflow.

Public project/activity snapshots contain opaque hashes rather than PID or full
filesystem paths. They never serialize raw argv, environment values, prompts,
tool arguments/results, transcripts, account identity, credentials, remotes,
or diff content. An inaccessible or unprovable cwd yields an Unassigned session
instead of basename, branch, port, or recent-activity guessing.

Memory Inspector resolves a fresh process snapshot from a recognized user-app
group. It does not accept a PID from the WebView. System processes, terminals,
and Zenith are protected. Normal application termination is offered before a
confirmed force termination because unsaved work can be lost.

Development Servers is a separate, narrower endpoint workflow. Discovery calls
`/usr/sbin/lsof` directly with fixed arguments and a timeout, parses its
machine-oriented output, and enriches current-user TCP listeners from a process
snapshot. Full command lines, environment values, and raw discovery output are
not returned or logged.

The frontend receives display metadata and a random, one-shot lease ID. Private
lease data includes the PID, protocol, port, bind address, UID, process start
time, executable identity, classification, and observation time. Leases expire
after 30 seconds, are capped in memory, and are consumed before any mutation is
attempted.

Before signaling, Rust requires the exact endpoint and stable process identity
to match a fresh snapshot and reruns the development-server classifier and
protected-process rules. Unknown ownership, missing identity fields, runtime
name alone, PID reuse, port handoff, privileged ports, and protected processes
fail closed. A normal release sends `SIGTERM` only to the exact listener PID.
Force release cannot be requested with an ordinary listing lease: it requires a
new force-authorized lease created only when the same process remains after the
grace period and a second user confirmation. If another process acquires the
port, Zenith reports an ownership change and never signals the replacement.

Local testing infrastructure is allowlisted with narrower executable checks.
`agent-browser` must resolve inside its official package binary directory.
Google Chrome for Testing must resolve to the exact testing app executable and
include both remote-debugging and isolated-profile arguments. Standard Chrome,
browser helper processes, crash reporters, and renamed lookalikes do not match.

## Failure behavior

Safety checks fail closed. A stale scan, missing signature, expired plan,
identity mismatch, unsupported manual operation, inaccessible path, failed
external command, stale Large Files inventory, or expired app inspection
produces an error and leaves the target untouched where possible. Cleanup events
report per-target results instead of converting a partial failure into a
success.

## Regression tests

Changes to a safety boundary require a temporary-fixture or pure-scope regression
test. The suite covers forged selections, manual-strategy rejection, nested
`.git` and declared exclusions, path traversal, protected roots, symlinks,
TOCTOU identity changes, intensive-mode opt-in filtering, protected cache
prefixes, typed model deletion, Docker total-versus-reclaimable accounting,
Large Files user-content scope, forged Large Files IDs, application-root scope,
app-data scope, development-server classification, lease expiry/one-shot
behavior, force authorization, PID reuse, and port ownership changes. Tests
must never point destructive operations at real user processes or directories;
the development-port integration test owns and cleans up its ephemeral child.

## AI Control Center safety invariants

AI Control Center enforces strict safety and privacy boundaries:

- **Canonical session identity:** Resource attribution consumes only the
  verified `AgentSession` and `ProjectIdentity` records from Project Cockpit.
  Unassigned or low-confidence processes remain visible for transparency but
  cannot authorize any mutable action.
- **Advisory-first automation:** Keep Awake automation is disabled by default,
  requires explicit policy opt-in, honors AC-only restrictions, treats unknown
  power as ineligible, and automatically releases its assertion when verified
  sessions exit. Recommendations never kill processes, close ports, or delete
  files automatically.
- **Opaque action previews:** Actionable recommendations generate opaque,
  expiring (120-second), one-shot preview tokens. Consuming a preview navigates
  the user to the corresponding review workflow; it never performs mutations
  directly.
- **Bounded safety inspection:** Project safety scans are user-initiated and
  strictly bounded to registered active project roots, a maximum of 2,000 files,
  1 MiB per file, and a directory depth of 8. Inspection never follows symlinks,
  skips cross-filesystem mounts, and never executes or rewrites tool configs.
- **Secret redaction guarantees:** Scans detect secret patterns (API keys,
  tokens, private keys) and broad MCP permissions, returning only the evidence
  type, relative path, and line numbers. Raw secret bytes, credentials, command
  arguments, headers, environment variables, and email addresses are never
  returned, logged, or persisted.
- **Git baseline integrity:** Captures a repository baseline on first session
  observation. Pre-existing uncommitted changes are excluded from change
  counts. Diffs are generated only upon explicit user request, bounded at 256
  KiB, and never persisted to disk.
- **Audit and telemetry:** Audit logs are local, bounded to 1,024 entries and
  512 KiB, sanitized before persistence, and retained for 1–365 days. Zenith
  collects zero telemetry or analytics. Full details are in
  [AI_CONTROL_CENTER.md](AI_CONTROL_CENTER.md).
