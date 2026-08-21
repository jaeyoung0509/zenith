# Cleanup Safety Model

Zenith deletes developer caches, so a correct UI is not considered a security
boundary. Every destructive decision is reconstructed and validated in Rust.

## Trust boundaries

The frontend may provide:

- a current `scan_id`;
- selected opaque item IDs;
- an opaque one-shot `plan_id` after reviewing a preview;
- typed identities for dedicated adapters, such as a freshly scanned model ID.

The frontend may not provide an executable path, cleanup strategy, risk tier,
filesystem identity, or arbitrary PID. These values are resolved from current
backend state.

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

Memory Inspector resolves a fresh process snapshot from a recognized user-app
group. It does not accept a PID from the WebView. System processes, terminals,
and Zenith are protected. Normal application termination is offered before a
confirmed force termination because unsaved work can be lost.

## Failure behavior

Safety checks fail closed. A stale scan, missing signature, expired plan,
identity mismatch, unsupported manual operation, inaccessible path, or failed
external command produces an error and leaves the target untouched where
possible. Cleanup events report per-target results instead of converting a
partial failure into a success.

## Regression tests

Changes to a safety boundary require a temporary-fixture regression test. The
suite covers forged selections, manual-strategy rejection, nested `.git` and
declared exclusions, path traversal, protected roots, symlinks, TOCTOU identity
changes, intensive-mode opt-in filtering, protected cache prefixes, typed model
deletion, and Docker total-versus-reclaimable accounting. Tests must never
point at real user directories.
