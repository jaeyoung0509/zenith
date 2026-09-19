# Cleanup Safety Model

Zenith deletes developer caches, so a correct UI is not considered a security
boundary. Every destructive decision is reconstructed and validated in Rust.

## Where authorization lives

Authorization is a property of the Rust application services, not of the IPC
surface. Tauri capabilities decide which window may call a command, and the
application service decides what that call is allowed to do; both answer
different questions, and neither substitutes for the other. A command that a
window is allowed to call still receives only opaque IDs, is still checked
against the backend's current scan or inventory, and still runs under the plan
TTL and one-shot rules below. `CleanupService`, `StorageService`, and
`SystemService` apply their capability gates and operation gates themselves, so
a capability file edited to be too generous cannot widen what a call may
achieve, and a capability file edited to be too narrow cannot make a service
skip a check.

Commands perform no filesystem mutation and store no workflow state. The
current scan, the plan store, the reviewed inventories, and the Trash executor
belong to the service that protects their invariant, and `DesktopState` exposes
services rather than those internals.

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
4. It names the cleanup unit that authorized it, and that path is the one the
   plan deletes.
5. The unit granularity and the ownership it reports are the ones the signature
   declares.
6. It is not a `Manual` target.
7. The plan is unexpired, has not been used before, and authorizes a mutation.
8. Its filesystem identity, entry kind, and modification record still match
   immediately before deletion.
9. It is not structured state, and it is not reached through a link, junction,
   reparse point, or mount boundary.
10. Every traversed entry passes blacklist and signature-exclusion checks.
11. It is not inside a unit the same plan already authorizes. Overlapping rules
    fold only when the broader observation is complete and their mutation
    policies are compatible; specialized provider/container/manual authority
    is never replaced by a generic filesystem target, and two rules that name
    the same location with different operations leave it unplanned rather than
    letting the wider one authorize the narrower one's deletion. An unresolved
    authority conflict blocks the broader item and keeps both observations
    visible (see
    [ARCHITECTURE.md](ARCHITECTURE.md#discovery-eligibility-and-cleanup-units)).

Before any of that runs, the executor classifies the target into the operation
it authorizes (`CleanupOperation::of`). Only the filesystem operation names a
path as mutation authority and only it can reach the tree deleter; a container
prune asks the runtime to prune what the runtime owns, and a provider prune asks
the tool to invalidate its own cache with fixed arguments, using the planned
location only to detect that the cache moved. A `Manual` resource classifies to
no operation and is refused, so there is no strategy-shaped fallback into a
filesystem mutation.

The tree deleter walks bottom-up without following symlinks. It unlinks a
symlink itself, preserves blacklisted or excluded descendants, and removes a
directory only after it is empty. Generic cleanup does not call
`remove_dir_all`.

On Unix, traversal holds verified directory descriptors opened with
`O_DIRECTORY | O_NOFOLLOW` and the final unlink for every file and directory
goes through `unlinkat` on the verified parent descriptor. A verified
descriptor is never converted back into an untrusted full-path lookup for the
final mutation, so replacing or redirecting any parent component after
validation cannot cause deletion outside the planned signature scope.

On Windows, directory and file identity is captured through handles opened
with `CreateFileW` using `FILE_FLAG_BACKUP_SEMANTICS |
FILE_FLAG_OPEN_REPARSE_POINT` and compared as stable volume/file IDs before
mutation. The verified handle does not share delete access, remains open
through validation, and applies final deletion with
`SetFileInformationByHandle(FileDispositionInfo)` to the same filesystem
object. A missing or zero `(device, inode)` identity is a verification failure,
never a skipped comparison. Reparse points and symlinks are never traversed;
symlink, reparse-point, and canonicalization failures fail closed on mutation
paths.

Execution-time freshness is fail closed by default: a planned file or
directory target whose modification timestamp changed after scanning aborts
with `ChangedSinceScan`. The narrowly documented exception is stale-temp
signatures with `min_age_days`, where the executor re-measures the full tree
newest-mtime immediately before deletion instead of relying on the single
directory mtime captured at plan time. Both the scan and the execution boundary
evaluate that rule through one function (`AgeObservation::evaluate`), so an
item's verdict and the guard's verdict cannot drift apart.

A target that changed in any of those ways is reported as `skipped`, not as a
failure: the run refused to delete an object the plan did not authorize, which
is the safe outcome, and the reason names what changed.

## Structured state

An age rule answers "has anything in here changed recently?". That is the right
question for a cache and the wrong one for a database, a lock file, a
credential store, a configuration file, an application bundle, or an executable.

`classify_structured_state` is the shared, name-shaped rule that draws the line.
It is pure — the caller supplies the entry kind and the executable bit, because
those come from metadata rather than from a name — and case-insensitive,
because Windows folds case by definition and a case-insensitive APFS volume can
spell the same name either way.

Three boundaries enforce it:

- the scanner records the classification on the item, so a discovered candidate
  is reported as `blocked` with the reason rather than becoming cleanable;
- the planner refuses a target whose root is structured state, so a plan never
  offers one; and
- the execution guard re-classifies immediately before mutating, so a path that
  became structured state after the scan is skipped instead of deleted.

Structured content *inside* a declared cache or log namespace is covered by
that namespace's contract: the unit is the application's regenerable cache
directory, and `delete_directory` removes it whole. What the classifier
prevents is a *discovery* rule — an age threshold, an enumerated child — from
manufacturing a target out of state that only its owner may invalidate. A
provider that legitimately owns a disposable database must go through its own
adapter with its own lifecycle, which is what `CleanupOperation::Provider`
exists for.

### A running owner

An application that is running keeps its own cache out of the default
selection. The scan asks the process table which application bundles are in
use — a process whose executable lives inside a `.app` bundle is running that
bundle — and a cache namespace named after a running bundle identifier (or one
of its children, such as `com.example.app.helper`) is reported as `reviewable`
with the reason rather than as `auto_cleanable`. The bytes are still counted,
and an explicit selection still cleans them: the rule removes the *default*
decision, not the user's.

A namespace whose owner is not running is judged by its age policy as usual. An
orphaned namespace — one whose application is gone — is the ordinary case, and
it needs no vendor list to be recognized: nothing in the catalog enumerates
which applications may own a cache.

### Unreadable roots

A location the process may not read is reported with the reason, never as an
absence: the item carries `quality = "unavailable"` and its `incomplete_reason`,
the scan counts it, and on macOS a refusal inside a protected location
(other applications' containers, Mail, Messages, Safari, the device-backup
store) names Full Disk Access as the setting that would change it. A scan that
could not read part of the machine says so instead of reporting a smaller
total.

## Absent targets and skipped results

A target that no longer exists when cleanup reaches it is already in the desired
state. The executor probes the target's own metadata before the canonical,
symlink, and identity guards — `exists()` is never used as that probe, because it
collapses a permission refusal into a silent success — and reports absence as
`skipped` with `NotFound` and nothing reclaimed. The same rule applies to a
nested entry that disappears mid-walk: it is skipped, not recorded as a
deletion failure.

A replayed plan therefore cannot delete anything: every target whose object is
gone or changed is skipped with a reason, and the replacement that now occupies
the path is left alone. Absence is a distinct signal
(`ZenithError::Missing`), not a variant of "changed since scan".

Every other failure keeps failing closed: an identity, mtime, size, symlink,
ownership, or permission failure still aborts the target that reports it, and
cleanup events still report per-target results instead of converting a partial
failure into a success.

## Result statuses

Per-target results answer "what happened to this object", not "did the call
return":

| status | meaning |
| --- | --- |
| `success` | the target's postcondition holds because this run removed it |
| `partial` | some of the target was removed |
| `skipped` | nothing was removed and nothing was wrong: the object was already gone, or is no longer the one the plan authorized |
| `failed` | the object was still the right one and Zenith could not remove it |

`success` is true for `success` and `partial` only. A skipped target removed
nothing, so it is never reported as cleaned; the reason
(`changed_since_scan`, `not_found`, `safety_boundary`, `structured_store`,
`blacklisted`) says which rule answered. Failure reasons distinguish
`permission_denied`, `in_use`, and `external_command_failed` from the skip
reasons, because those are the ones a user can act on.

Each result keeps two byte populations apart: `estimated_bytes` is what the scan
measured and the plan was built from, and `bytes_reclaimed` is what this run
measured after mutating. A skipped or failed target never reports an estimate as
reclaimed.

## Platform coverage

Safety guarantees are equivalent in policy and testable on both platforms,
within the limits CI actually executes:

- Unix (`macos-latest`, `ubuntu-latest`): descriptor-relative `unlinkat`
  deletion, parent-replacement race tests, symlink escape tests, and
  permission/ownership checks run in CI.
- Windows (`windows-latest`): no-follow handle identity capture, `(0, 0)`
  rejection, reparse-point (junction) non-traversal, case-insensitive
  protected-path checks, and long (`\\?\`-prefixed) path handling run in CI
  using temporary fixtures only. Windows graceful process termination is
  reported as unavailable rather than mapping `TerminateProcess` to graceful.
- No destructive test points at real user processes or directories.

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
registered signatures marked `intensive_only`. A signature that opts into
always-on discovery (`discovery = "always"`) is inventoried in either mode:
the scope then decides eligibility, not visibility, and a unit it finds in
standard mode is reported as `policy_gated` and is never selectable.

Broad user cache and log signatures are constrained as follows:

- only direct children of the registered root can become targets;
- the root itself is never returned as a cleanup item;
- symlink children and protected Apple/system prefixes are skipped, with prefix
  exclusions matched case-insensitively so a namespace cannot be admitted by
  casing alone;
- a cache namespace whose owner publishes its own download, validation, and
  invalidation command is excluded rather than treated as a generic cache; and
- the newest timestamp anywhere in the candidate tree is compared against the
  declared minimum inactivity age: an older candidate may be cleaned, and a
  newer one is reported as `recent` with the bytes it holds;
- incomplete traversal, permission failure, or recursion depth cutoff excludes
  the candidate;
- the planner accepts only a direct child of the resolved signature root; and
- the executor repeats the full-tree inactivity check immediately before
  deletion and aborts if anything became recent.

The current thresholds are seven days for third-party children under
`~/Library/Caches`, for the application cache subtrees under
`~/Library/Application Support/*/{Cache,Code Cache,GPUCache,...}`, for the
system per-user cache root (`${DARWIN_USER_CACHE}`), and for the Windows
application and browser caches; fourteen days for application-log groups under
`~/Library/Logs`, for sandbox and group container caches, and for an
unrecognized entry in `%TEMP%`; and thirty days for Xcode device support.
Diagnostic and crash-report groups remain protected, as are the Apple cache
namespaces and the tool-managed namespaces named in the signature, such as
`dotslash` and `ms-playwright`.
Intensive mode does not scan user documents, preferences, credentials,
databases, model weights, unknown `/tmp` children, or any Windows-owned
maintenance store.

### Age policies

Two shapes of age policy exist, and the difference is what the policy is about:

- **A whole unit.** The newest timestamp anywhere in the unit's tree must be
  older than the threshold, or nothing in it is removed. This is the policy for
  a cache an application abandons as a whole.
- **The entries inside a unit** (`strategy = "delete_stale_contents"`). Each
  entry is judged on its own timestamp, and only the entries that satisfy the
  policy are removed. This is the policy for a namespace that is written to
  while it is being cleaned: one file touched this morning no longer hides the
  gigabytes beside it, and the file that is being written stays.

The second shape is still a per-entry decision at execution time, not a
tree-level verdict applied to leaves: `StaleEntryPolicy` evaluates one entry
from its own name, kind, and timestamp, the scan measures with it and the
execution guard re-evaluates it for every file immediately before that file is
unlinked. An entry whose age cannot be read is not an old entry, and an entry
that is structured state is never stale-deletable however old it is — the
database, write-ahead log, lock, credential, configuration file, bundle, or
executable rules apply per file here exactly as they do at a unit root.

A directory inside such a unit is descended into (never through a link) and
disappears only when it ends up empty, so a namespace keeps whatever it still
holds.

### Selectors

A catalog root may contain two kinds of pattern: `*`, which matches one
directory name, and `{a,b,c}`, which matches one of those names. Nothing else is
a selector — `Cache*`, `**`, and nested braces are refused when the catalog
loads, because a pattern that silently matches nothing is an entry that
silently does nothing.

A selector is allowed in `paths` only. `exclusions`, `include_prefixes`, and
`exclude_prefixes` are matched against concrete names or paths, so selector
syntax there is a load error rather than a pattern that protects nothing.

Expansion is bounded (256 matches per pattern, with a recorded diagnostic when
the cap is reached), deterministic (sorted, matched case-insensitively only on
a platform that folds case), and never descends through a link: a selector
component is resolved with `symlink_metadata`, so a symlinked application
directory is not enumerated as a root. The placeholder expander runs first, so
`${LOCAL_APP_DATA}/Packages/*/TempState` resolves the root and then selects
within it.

Whether a match is a cleanup unit or a root whose children are units is the
signature's `unit` declaration: `named_subtree` treats each match as one object
that is aged and deleted whole, `child_namespace` ages each child separately.
The unit you age is the unit you delete, in both shapes.

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

A cleanup `DeletePlan` is not a reviewed Trash plan. `DeletePlan` authorizes
removing signature-scoped caches inside the scan that produced it, and its
identity check is `CleanupIdentity`. A reviewed Trash plan authorizes moving a
user-selected file or app to the Trash, and its evidence is
`ReviewedFileIdentity` plus the reviewed scope that produced the target. They
are separate types with separate evidence requirements (and separate plan
kinds in one store), so a reviewed target can never satisfy a cleanup check and
a cleanup plan can never reach the Trash port. The trust models stay separate
because the user's consent is different in each: one is "clean this cache",
the other is "move this file I chose".

Trash plans and cleanup plans share one bounded, expiring, one-shot store:
both expire after five minutes, both are capped at 64 entries with oldest-first
eviction, and both are removed the moment they are taken — whether the run then
succeeds or fails — so an ID authorizes at most one mutation. Every target is
revalidated immediately before its individual Trash operation, and the port
accepts only the `ApprovedTrashTarget` that validation produces, so a move
cannot be requested with a path that skipped the checks. Partial failures are
reported per item.

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

## Tool-managed cache safety

Package stores and AI runtime roots are not authorized merely because their
paths are known. When an owner exposes locking, garbage collection, revision
selection, or project-aware purge, Zenith must use a typed fixed-argument
adapter or remain advisory. `external_command` targets rediscover and
canonicalize their path immediately before mutation, require current-user
containment, reject symlinks/reparse points and untrusted executables, and fail
if the owning process is active or the location changed. Frontend values can
never select executables, arguments, paths, environment variables, packages,
or model revisions.

GPU/JIT signatures cover only independently rebuildable per-user defaults.
Driver packages, ProgramData, WSL VHDX files, container volumes, models,
datasets, optimized engines, performance databases, prompt/session state, and
mixed runtime roots remain out of generic deletion. Allocated bytes for shared,
hard-linked, cloned, sparse, or deduplicated stores are labeled as a lower
bound rather than promised reclaimed space.

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
