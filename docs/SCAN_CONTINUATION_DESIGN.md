# Budgeted scan continuation — #238 design scaffold

**Status: proposed; no runtime implementation or IPC changes in this PR.**

This is the remaining scan-lifecycle work from [#238](https://github.com/jaeyoung0509/zenith/issues/238),
after [#239](https://github.com/jaeyoung0509/zenith/pull/239). The reviewed baseline
is `develop` at `a76237e3bf6d0cf9ab53df8ccaa5e0a5bbf045fc` (2026-09-21).
Owner-scoped Cargo providers, paged selector traversal, and eligibility/refusal
handling already exist there; do not build another provider framework.

This document fixes the boundaries, failure behavior, and implementation order.
Names below describe proposed contracts, not types or commands already available.
No unused production stubs, extra dependencies, generated bindings, or feature
flags are needed for this design-only step. #238 remains open.

## 1. Scope and decisions

The current scanner consumes selector pages within one request. A page limit
bounds a page, not the total work of a scan. The missing capability is to stop
at a safe checkpoint and continue the **same backend-owned inventory** in a
later request, without dropping earlier observations or restarting at page one.

Keep these concerns separate:

- **Admission/concurrency:** reuse `ExecutionBudgets` and the shared scan pool.
- **Slice limits:** bound root admissions and selector-page requests per pass.
- **Lifetime:** bound retained state and revoke stale continuation tokens.
- **Observation quality:** exhausted traversal can still have permission or
  measurement gaps. Exhausted does not mean every byte was observable.

Discovery never authorizes cleanup. A continuation grants only the right to
resume discovery; it is not a delete plan and cannot weaken provider, scope,
identity, age, process, link, or structured-state checks.

## 2. Ownership and extension points

| Existing seam | Responsibility for the follow-up |
|---|---|
| [`zenith-core::domain::scan`](../crates/zenith-core/src/domain/scan/mod.rs) | Framework-independent slice-limit and completion semantics. Keep platform cursors out of core. |
| [`zenith-platform::selector`](../crates/zenith-platform/src/selector.rs) | Continue using `SelectorCursor` and `ContinuationLost`; own native enumeration and cursor validation. |
| [`ScanEngine`](../src-tauri/src/scanner/engine.rs) | Yield a backend checkpoint, seed prior raw observations, and finalize one merged inventory through existing overlap/eligibility logic. |
| [`ScanService`](../src-tauri/src/services/scan_service.rs) and [`ScanStore`](../src-tauri/src/services/scan_store.rs) | Own start/claim/publish/invalidate as one lifecycle. Compose the coordinator once, rather than creating a continuation map per command. |
| [`ExecutionBudgets`](../src-tauri/src/execution_budget.rs) | Continue bounding expensive execution. Do not reinterpret semaphore permits as scan coverage limits. |
| [`scan DTOs`](../crates/zenith-core/src/application/dto/scan.rs) and [`frontend scan store`](../src/lib/stores/scan.svelte.ts) | Project the published inventory and resumability. The UI never reconstructs cursors or merges authoritative totals. |

Follow the existing application-service gate/admission discipline. No store lock
may be held while waiting for permits, doing filesystem work, emitting events,
or calling providers. Keep Tauri commands as transport adapters.

## 3. Minimal contracts

The implementation needs these shapes, not a generic workflow framework:

```text
SliceBudget = { max_roots: NonZero, max_selector_pages: NonZero }

SliceOutcome =
    Exhausted { observations }
  | Paused    { observations, checkpoint, limit }
  | Stopped   { observations, reason }

ResumeRequest = { scan_id, continuation_id }

PublishedScan = {
    result: ScanResult,
    discovery: Exhausted | Paused { continuation_id } | Stopped { reason }
}
```

`Paused` is a published state only after its checkpoint is accepted by the
backend store. Failure to retain it becomes `Stopped` with an explicit reason,
not a token-shaped placeholder. The caller cannot supply a path, a provider,
a selector cursor, a seed result, or a replacement request on resume.

### Honest limits

Charge the budget **before** admitting a root or requesting another selector
page, including a page that finds no matching roots. Validate nonzero limits;
never create an endlessly resumable pass that can do no work. Admit no more
parallel roots than the remaining root allowance; count admitted work even
when it produces a refusal or no candidate.

A fetched page may contain more matches than this pass can measure. Retain its
unconsumed roots, in order, together with its continuation. Advancing the cursor
without those pending roots loses coverage. Replaying the whole page to avoid
storing them is not a valid substitute.

**First increment: root/page admission bounds, not hard entry or time bounds.**
A root measurement may still be large, and producing or validating a selector
page can enumerate directory names to compute its digest. A 256-parent page
must not be described as at most 256 filesystem reads. Cancellation remains
cooperative. Finish admitted root measurements before checkpointing; do not
add mid-root resume to this increment.

A later visited-entry or wall-clock work budget needs resumable enumeration
and walker frontiers, plus progress accounting for that work. Do not expose
such a guarantee until those adapters and tests exist. Choose production limit
values from measurements; this document deliberately specifies no magic values.

## 4. Backend checkpoint and result identity

Keep one typed, non-serializable checkpoint containing:

| Part | Required information |
|---|---|
| Frozen context | Original `ScanRequest`, catalog/provider configuration generation, and the injected platform environment identity relevant to scope. Context changes require a new scan. |
| Identity | Backend session/generation, latest published `scan_id`, and the revision the next pass is allowed to replace. |
| Frontier | Category/signature/selector position, existing `SelectorCursor`, and bounded pending roots from a fetched page. Bind cursors to their selector and root provenance. |
| Accumulator | Prior **raw observations**, stable unit/provenance keys, gaps, and measurement timestamps needed for finalization. |
| Lifetime | Original freshness deadline, idle expiry capped by it, and retained-size accounting. Never extend the original deadline by resuming. |

The backend session is stable across passes. Each published result has a new
`scan_id`; existing scan/plan validation must reject IDs from an older revision.
The invocation ID used by progress and cancellation identifies the current
pass, not a reusable cleanup authorization.

### Merge before reducing

Seed the next pass from retained raw observations, add newly completed work,
and run the existing overlap resolution, eligibility derivation, category
aggregation, and gap aggregation over that combined set. Return a replacement
snapshot, not a byte delta for the frontend to add.

Do not seed only from the previously displayed, overlap-suppressed result:
a later parent or conflicting authority may change how an earlier child is
represented. Use stable unit identity **and signature/root provenance**, not
row position or path text alone. Replayed progress and duplicate work must not
count twice; distinct authorities must not be silently folded together.

For a stable fixture, the resumed result must equal an uninterrupted scan for
items, authority decisions, byte populations, and persistent gaps. Per-pass
IDs and timing metrics naturally differ. Budget-pause metadata disappears when
work is exhausted; permission/I/O gaps do not. Totals need not increase
monotonically when a newly discovered overlap changes accounting.

Retaining an observation does not make it newly measured. Enforce an absolute
session freshness deadline in the authoritative cleanup validation path;
a new `finished_at` or `scan_id` must not refresh old observations. Keep the
store's expiry clock monotonic and inject a clock for tests. If expiry is
reached during work, do not publish the merged snapshot as fresh/cleanable.

## 5. Lifecycle: one authority, one-shot claims

The current `ScanStore` owns the latest trusted scan. Extend that lifecycle
rather than placing an independent token mutex beside it: validating a snapshot
and consuming its continuation must be one atomic transition. Publication must
atomically install both the new snapshot and its optional token.

```text
new scan -> Running -> Exhausted
                   -> Paused(token) -> claim once -> Running -> ...
                   -> Stopped(reason)

new generation / cleanup / expiry / context change -> revoke continuation
```

**Start.** Allocate a new generation, supersede previous continuation state,
and cancel superseded work. A late worker can finish physically but cannot
publish over a newer generation. There is only one latest inventory today;
do not introduce multi-session browsing as part of this change.

**Claim.** Under the lifecycle lock, check token, current `scan_id`, generation,
context, and expiry, then consume the token exactly once. A racing caller
cannot claim the same checkpoint. Mark the previous revision non-current for
cleanup while the resumed pass runs. Release the lock before doing work.
Do not consume a valid unrelated token when rejecting a mismatched request.

**Publish.** Commit only if the worker still owns the expected generation and
revision. Finalize the seeded observations and publish a fresh one-shot token
only when there is remaining work, the lifetime is valid, and retained-state
limits allow it. Exhaustion publishes no token. A superseded worker discards
its result rather than restoring an old scan or continuation.

**Stop/failure.** Explicit cancellation is terminal for this increment: return
truthful partial observations without a token; do not disguise it as a budget
pause. A claimed token is never reinserted after failure. The previous display
may remain for information but cannot retain cleanup authority. Unexpected
worker failure must release its claim/state through the existing error path.

**Mutation.** Coordinate continuation invalidation with
`validate_and_invalidate_for_cleanup` and the existing operation gate. An
accepted destructive operation revokes the inventory generation before
mutation; in-flight discovery cannot restore it afterward. Review every other
inventory-invalidating mutation through the same boundary. Merely preparing a
read-only preview does not itself mutate files or grant a continuation.

### Bounded, ephemeral storage

Use backend-generated unpredictable opaque IDs, the existing UUID facility
where suitable, and no token logging. Keep checkpoints in memory only; app
restart requires a fresh scan. Do not serialize backend authority state to
preferences or expose it through IPC.

The coordinator needs a hard entry cap, an original session deadline, idle
expiry, and limits on retained observations, pending roots, and path/string
storage. Count claimed/in-flight checkpoints too, not just idle map entries.
Expiry and invalidation must drop their retained state. Recover a poisoned
lock by discarding disposable state and requiring a fresh scan, never by
trusting a potentially half-transitioned checkpoint.

If retention is refused, preserve the partial-result explanation, with no
Continue action. At this scope, rejecting retention is simpler than a general
LRU/session system. Unknown, used, expired, and discarded tokens may all map
to a typed `ContinuationUnavailable` refusal; do not retain unbounded tombstones
just to distinguish them. A stale scan or changed context should also be a
typed refusal, not inferred by matching error-message text.

`SelectorCursor` validates a name-enumeration digest; that is not a deletion
authority or a complete filesystem identity proof. Continue applying native
link/root identity checks even when names match. `ContinuationLost` must stop
with a coverage gap and no token. Never silently restart the traversal while
calling the operation a continuation.

## 6. Interface projection

| Published state | User-visible behavior |
|---|---|
| Exhausted | No Continue action. Render retained permission/measurement gaps independently of discovery completion. |
| Paused with retained token | Show Continue scan; send exactly the published scan ID and opaque token. Disable repeated submission while the request is in flight. |
| Stopped, cancelled, expired, or unavailable | No Continue action. Explain the interruption and offer a clearly labelled new scan where appropriate. |
| Superseded revision | Ignore late results/events and discard old plan IDs; never make old rows current again. |

The backend remains authoritative when a displayed token expires or another
window wins the claim. Handle that refusal by clearing local resumability,
not by automatically restarting and pretending progress was preserved.
Progress is provisional; accept the backend's final merged snapshot. Do not
restore selections or plans by row index across revisions.

Project new transport types under `application::dto`, register the command and
only necessary window capabilities, regenerate bindings, and update native
and deterministic browser mocks together. Every serialized `u64` still needs
the shared `ipc_numeric` adapter and Specta annotation. Do not hand-edit the
generated TypeScript or widen Quick Panel permissions by default.

## 7. Required acceptance tests (planned, not executed)

| Boundary | Acceptance criterion |
|---|---|
| Page tail and budget | A fixture larger than `SELECTOR_PAGE_LIMIT`, with fewer allowed roots than page matches, resumes through every pending root once; exact-boundary and empty-result pages terminate correctly. |
| Honest admission | Root/page allowances are never exceeded, including parallel dispatch. No claim is made about bounding entries inside one root or selector digest computation. |
| Merge correctness | Resumed and uninterrupted fixtures agree, including cross-pass overlaps and conflicting authorities. Replayed events do not inflate bytes; permission gaps survive completion. |
| Cursor invalidation | Insert/remove/rename before the saved position yields `ContinuationLost`. Root replacement or link substitution fails native identity/safety checks even if names are unchanged. |
| One-shot/concurrency | Two windows claiming one token produce one worker. A late worker cannot publish after a new scan or cleanup invalidates its generation. |
| Lifetime/resources | Injected-clock tests cover expiry at the boundary, expiry during work, cap/retained-size refusal, poisoned state, cancellation, and worker failure without leaks or resurrected tokens. |
| Freshness/cleanup | Repeated resume never extends the original freshness deadline. Old scan/plan IDs fail, and a continuation cannot authorize deletion. |
| UI/IPC | Continue exists only for a retained token; stale responses cannot replace current state. Native/mocked shapes and generated numeric bindings agree. |

Use temporary fixtures only, and the shared benchmark for measurement rather
than real user caches. These tests are exit criteria for future implementation,
not claims of coverage supplied by this documentation PR.

## 8. Delivery order and activation gate

1. **Core semantics + lifecycle.** Add only the concrete slice types and
   coordinator needed by the engine. Test claim/publish/invalidate, freshness,
   and retention boundaries with injected time. Keep current scan behavior.
2. **Engine checkpoint/resume.** Implement pending-root preservation, seeded
   raw accumulation, and finalization. Prove resumed/uninterrupted equivalence
   and stale-worker rejection before enabling a production budget cutoff.
3. **IPC + UI.** Expose the tested workflow, regenerate bindings, add browser
   mocks, and wire truthful Continue/new-scan behavior. Measure before choosing
   default limits. Run the normal Rust/frontend checks and platform validation.

**Do not enable early budget termination without the corresponding tested
resume path.** Do not close #238 on this design PR. Hard entry/time budgeting,
mid-root walker continuation, persisted sessions, multi-session UI, provider
expansion, and cosmetic polish remain outside this scaffold.
