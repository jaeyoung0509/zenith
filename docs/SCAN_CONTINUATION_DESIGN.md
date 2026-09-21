# Budgeted scan continuation — #238

**Status: implemented by PR #247.**

This is the remaining scan-lifecycle work from
[#238](https://github.com/jaeyoung0509/zenith/issues/238), building on the
owner-scoped Cargo providers, complete selector traversal, and typed
eligibility/refusal handling delivered by
[#239](https://github.com/jaeyoung0509/zenith/pull/239).

## Scope

Selector expansion already consumes deterministic, bounded pages until every
root is reached. A page limit bounds memory and work in flight; it does not
truncate coverage. The continuation added here bounds a full dashboard scan at
category boundaries: the main window admits two categories per pass, publishes
the observed lower bound, and retains the remaining categories behind one
opaque, one-shot token. The next request resumes after the completed categories
and publishes a fully recomputed replacement snapshot.

This first production budget deliberately does not stop midway through a root,
signature, or selector page sequence. Once a category is admitted, its selector
pages and root measurements run to a safe boundary. A later entry-count or
wall-clock budget would require walker-frontier persistence and is not claimed
by this implementation.

The Quick Panel is not granted the resume command. Its scans run to exhaustion
in one request, and it will not adopt a paused main-window snapshot.

## Contracts

The interface sees one backend-owned projection:

```text
PublishedScan = {
  result: ScanResult,
  discovery:
      Exhausted
    | Paused { continuation_id }
    | Stopped { reason }
}

ResumeRequest = { scan_id, continuation_id }
```

The caller cannot submit paths, categories, prior results, provider IDs,
selector cursors, or cleanup authority on resume. A continuation grants only
the right to continue discovery.

The private checkpoint retains:

- the original validated `ScanRequest`;
- the categories not yet admitted;
- prior slice results used as raw category observations; and
- the first publication's freshness anchor.

Every new pass receives a new invocation/scan ID for progress and cancellation.
The backend merges all retained categories, reruns cross-category overlap
resolution, and recomputes every aggregate. The frontend receives a replacement
snapshot and never adds byte deltas.

## One authority boundary

`ScanStore` owns the latest snapshot, generation, revision, running lease, and
optional continuation under one lock:

```text
new scan -> Running -> Exhausted
                   -> Paused(token) -> claim once -> Running -> ...
                   -> Stopped(reason)

new generation / cleanup / expiry -> revoke continuation
```

- **Start:** increments the generation and revokes the previous snapshot and
  token before work begins.
- **Claim:** atomically validates `scan_id`, token, generation, revision, and
  expiry, then consumes the token. A mismatched request consumes nothing and
  racing requests produce at most one worker.
- **Publish:** succeeds only for the worker that still owns the generation and
  revision. A late worker cannot restore superseded state.
- **Mutation:** cleanup validation accepts only an exhausted snapshot and
  atomically invalidates its generation before mutation.
- **Failure/cancellation:** never reinserts a consumed token. The retained
  display becomes `Stopped` and has no cleanup authority.

The lock is released before permits, filesystem work, events, or provider
calls. Poisoned disposable state is cleared instead of recovered as trusted
authority.

## Lifetime and retention

Continuation IDs are unpredictable UUIDs, in-memory only, and never logged.
Only the latest scan can retain one continuation, which is a hard entry cap of
one. Retention also refuses checkpoints above 50,000 items or 8 MiB of retained
item identity/path text.

The first published result owns the absolute freshness deadline. Resuming does
not move `finished_at`, so repeated passes cannot renew cleanup authority. A
claim at or after the deadline is refused and clears the token. App restart,
new scan, cleanup, cancellation, worker failure, or retention refusal requires
a new scan.

## Interface behavior

- `Exhausted`: cleaning may use the current result, subject to the existing
  per-item eligibility and freshness checks.
- `Paused`: cleaning is disabled and the main window shows **Continue Scan**.
  Repeated submission is coalesced by the frontend and rejected one-shot by the
  backend.
- `Stopped`: no Continue action or cleanup authority remains; the reason is
  shown and the user may start a new scan.
- A stale response cannot replace a newer generation, and a resumed snapshot
  does not restore selection by row index.

The command is registered only in the main-window capability. Native bindings
and deterministic browser mocks share the same top-level response shape.

## Verification boundaries

Regression coverage proves:

- invalid token attempts do not consume the valid token;
- a token can be claimed only once;
- a late worker cannot publish after a new generation;
- resumed publication preserves the original freshness anchor;
- a four-category scan advances through two bounded passes and produces each
  category exactly once in order;
- cleanup authority is unavailable until discovery is exhausted;
- cancellation and failures do not resurrect continuation state; and
- the frontend disables cleanup while paused, submits the published IDs, and
  enables cleanup only after an exhausted replacement snapshot arrives.

Selector regression fixtures independently cover more than 256 wildcard
parents, later-page matches, deterministic ordering, exclusions, symlink
refusal, and `ContinuationLost` when a selector namespace changes during its
paged traversal.

## Deliberate non-goals

- persistence across app restarts;
- multiple simultaneously browsable scan sessions;
- mid-root or mid-signature continuation;
- hard visited-entry or wall-clock guarantees; and
- widening Quick Panel permissions.

Those features require additional bounded walker state and must not be inferred
from this category-slice implementation.
