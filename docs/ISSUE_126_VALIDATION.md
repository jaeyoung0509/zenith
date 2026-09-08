# Dashboard freshness and numeric layout (#126)

## Diagnosis

The reported screenshot shows a nine-hour-old scan. The exact original error
text was not supplied, so it cannot be attributed to one specific filesystem
failure. Inspection confirmed the stale-result path:

- `ScanStore.isStale()` was only consulted at initial dashboard/quick-panel
  activation. Elapsed time was not reactive, and `cleanItems()` did not check it.
- The generic planner checked scan identity, but not observation age. A cached
  scan could therefore remain apparently actionable until a changed/missing
  target, replaced scan ID, or expired deletion lease caused a later failure.
- Main scan observations and dedicated storage inventories are different
  contracts. Dedicated inventory/plan TTLs remain 15/5 minutes. Generic scan
  observations now enforce the existing frontend's five-minute freshness policy
  in Rust as well, with the lifetime included in the typed response.
- The primary disk summary and mounted-volume list used separate IPC requests
  and activation lifecycles. They now derive from the same volume response.
- Physical Memory combined both values and units in one wrapping text node.
  In an isolated reproduction of the previous 182px content area, changing
  `9 GB / 16 GB` to `10 GB / 16 GB` increased text height from 32px to 64px.

## Behavior and safety

- Explicit empty/fresh/stale/refreshing/failed state; old results remain visible
  as historical data, but old selections are cleared and cleanup is disabled.
- Visible subscribers own one local clock timer. Hidden views do no freshness
  polling; focus/resume revalidates the backend cache without running a scan.
- Manual rescan uses a shared in-flight request. A delayed cache response cannot
  overwrite a newer scan. Failures retain historical data without authorizing it.
- Rust validates ID and age both during planning and immediately before execution.
  Preview expiry is capped by the observation's remaining lifetime. Execution
  invalidates the cached observation, including for partial cleanup, so another
  webview cannot reuse it. One-shot plans, scope checks, TOCTOU and permissions
  remain enforced. No failed cleanup is automatically retried.
- Category details resolve against the new inventory instead of retaining a
  captured category object from an older scan.
- ByteValue keeps each number/unit pair together. Physical/swap totals use a
  separate stable line; process metrics and actions have reserved columns.

## Reproducible verification

Automated tests:

- `scanFreshness.test.ts`: TTL boundary, backward clock, stale click without a
  timer tick, backend restart, scan coalescing, late response, failed refresh,
  expiry during planning, actionable error recovery, hidden/subscriber lifecycle.
- `diskSnapshot.test.ts`: two overlapping callers produce one IPC and one
  consistent snapshot; failed refresh preserves the previous complete snapshot.
- `numericLayout.test.ts`: number/unit DOM contract, Korean labels, fixed metric
  and action columns for both protected and terminable processes.
- Rust scan model tests: exact TTL, unknown ID, clock rollback, serialized policy.
  Existing filesystem regression tests cover missing/changed targets, permissions,
  symlinks and deletion boundaries using temporary fixtures.

Browser preview procedure (never run the fixture script in the native app):

1. Start `pnpm dev --host 127.0.0.1 --port 1421`.
2. Open the local preview in `agent-browser`, select Memory, and wait for metrics.
3. Run `agent-browser --session zenith126 eval --stdin < scripts/verify_numeric_layout.js`.
4. Repeat at 960×660 and 800×660, with expanded/collapsed sidebar and 125% CSS
   zoom. Reload after HMR before measuring so module instances match the page.

The script exercises seven values across digit and MB/GB transitions, including
100 and 1023.9 GB, and long Korean process labels. It fails on absent fixtures,
number overflow, or changing card/metric/action geometry. Baseline 960×660 and
800×660 measurements showed 0px geometry movement; the old text-only reproduction
showed 32px movement. At 125% zoom the dashboard now fits its container rather than
scaling a fixed viewport-width child beyond the visible window.

A browser fixture with `finished_at` set nine hours into the past displayed the
stale notice, cleared selections, and disabled Clean Safely. Clicking Scan Again
restored a fresh reviewed selection without executing cleanup.

Performance scope: initial storage summary + volume collection changes from two
independent disk IPC calls to one shared volume request per refresh. Concurrent
refreshes/scans share their promise; no continuous filesystem scan was added.
This is a request-count/layout improvement, not a claim of measured native CPU
or battery savings. Windows native checks run in PR CI; browser screenshots do
not prove native WebView2 or physical sleep/resume behavior.
