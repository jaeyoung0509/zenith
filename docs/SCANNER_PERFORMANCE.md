# Scanner performance and validation

What the cleanup scan is allowed to do, what it reports about what it did, and
how a regression is detected. The contract here is enforced by tests in
`src-tauri/src/scanner` and by the benchmark in
`src-tauri/tests/scan_benchmark.rs`; the platform-specific validation lives in
[WINDOWS_VALIDATION.md](WINDOWS_VALIDATION.md) and [SAFETY.md](SAFETY.md).

## Traversal bounds

Every walk runs under one stated set of limits
(`scanner/observation.rs`, `ScanLimits::default`):

| Bound | Value | What it bounds |
|---|---|---|
| `max_depth` | 32 | Directory levels below a root. A deeper entry is counted and reported as the reason a measurement is incomplete — never silently skipped. |
| `max_concurrent_directory_reads` | 16 | Directory tasks a walk may keep outstanding at once. |

The work executes on one process-wide, explicitly bounded Rayon pool
(`execution_budget::shared_scan_pool`, at most four workers, capped by the
machine's performance cores on macOS). The limits above bound the *queue*, not
the threads: past `max_concurrent_directory_reads` the walk descends inline on
the worker that is already running, so a tree of a million directories is work
for four threads rather than a queue of a million tasks. Both numbers are
stated once and threaded through `WalkContext`, so a walk cannot pick up the
bound in one path and miss it in another.

The benchmark asserts the bound as a measurement: for every fixture the run
reports `peak_outstanding_directory_tasks <= max_concurrent_directory_reads`,
and the wide fixture reaches the bound rather than trivially satisfying it.
The inline and pooled schedulers are also required to report the same traversal
facts; scheduling changes execution, not the meaning of `visited_entries`.

### Determinism

Two scans of one tree state the same items in the same order, with the same
bytes, the same skipped counts, and the same incomplete reason. Reasons are
chosen by the walk's own order — shallowest failure first, then path, then
message — rather than by whichever worker finished first
(`scanner/size.rs`, `FailureRecord`), so a parallel walk and a sequential walk
of the same tree report identical results.

## What a scan reports

`ScanResult.metrics` (`ScanMetrics`) carries the run's own measurements:

| Field | Repeats across scans? | Meaning |
|---|---|---|
| `visited_entries` | yes | Files and directories the traversal looked at, including the ones it refused. |
| `directories_read` | yes | Directories whose contents were read. |
| `duration_ms` | no | Wall-clock duration of this run. |
| `peak_outstanding_directory_tasks` | no | The highest number of directory tasks this run kept outstanding. |

`ScanResult.cancelled` states whether the run stopped because it was cancelled;
a cancelled scan is `quality: partial` with a stated reason, and the flag is
what lets the interface say *which* kind of incompleteness it is.

### Progress

`ScanEvent::RootStarted { category, signature_id, name, root }` is emitted once
per root, before the walk reads it. A scan spends most of its time inside one
root, so naming the root is what lets the interface show where a long scan is;
per-category item counts arrive with `CategoryFinished`.

### Cancellation

`cancel_scan(scan_id)` sets the signal the scan registered under the id its
`Started` event carried. The probe is consulted at every category, signature,
directory, and entry boundary, so a cancel stops the walk at the next boundary
and the partial result says so. The registry that owns those signals has a TTL for abandoned bookkeeping and a
hard entry cap. A still-running scan keeps another reference to its signal, so
age alone cannot make its Stop control expire; entries are removed on success,
on cancellation, and on error (`services::CancellationRegistry`).

Cancellation latency — the time from the request to the scan returning — is
measured by the benchmark (`cancellation_latency_is_measured_from_the_first_candidate`),
which also asserts the work actually stopped: the cancelled run reads fewer
directories than the fixture contains and never produces the later candidates.

## Benchmark and regression guard

```bash
# Run the harness and print this machine's metrics table.
cargo test -p zenith-desktop --test scan_benchmark -- --nocapture

# Regenerate the committed baseline after adding or changing a fixture.
cargo test -p zenith-desktop --test scan_benchmark -- --ignored --exact export_scan_baseline
```

`src-tauri/tests/fixtures/scan-baseline.json` holds the deterministic facts per
fixture — visited entries, directories read, candidate count, skipped entries,
allocated bytes — plus the stated bounds and a generous duration ceiling. CI
regenerates the file on both platform jobs and fails on a diff, exactly like the
TypeScript bindings; the same jobs print the metrics table so a real-machine
baseline can be read out of a log.

A regression is therefore caught two ways: a **count** that moves (exact
comparison — a scanner that walks more or less than it did is a defect, not a
slow run) and a **duration** that exceeds its ceiling (a bound with at least an
order of magnitude of headroom, because CI runners are shared and slower than a
developer machine). Timings are deliberately not compared byte-for-byte.

Fixtures currently covered: `wide` (200 directories), `deep` (past the depth
limit), `mixed_size` (3 B to 2 MiB), `mixed_age` (stale and fresh siblings),
`overlapping_roots` (one location, two rules), plus unix-only `inaccessible`
and `symlink` fixtures that are asserted by their own tests.

## Recorded real-machine baselines

Counts are properties of the fixtures and are identical on every machine; the
numbers below are what the machines measured, recorded so a future run can be
compared against a known point rather than against a feeling.

| Machine | OS build | Date | Zenith | Baseline |
|---|---|---|---|---|
| MacBook Air (Apple M1, 8 cores) | macOS 27.0 (26A428), Darwin 27.0.0 | 2026-09-19 | 0.3.36 | the table below |
| MacBook Air (Apple M1, 8 cores, 16 GiB) | macOS 27.0 (26A428) | 2026-09-21 UTC | 0.3.45 | [synthetic and live evidence](validation/2026-09-21-macos-0.3.45.json) |
| MacBook Air (Apple M1, 8 cores, 16 GiB) | macOS 27.0 (26A428) | 2026-09-22 UTC | 0.3.48 | [filesystem, provider, container, and cancellation evidence](validation/2026-09-22-macos-0.3.48.json) |

macOS, `cargo test -p zenith-desktop --test scan_benchmark -- --nocapture`:

```text
fixture            duration_ms  visited  directories  peak_tasks  candidates  skipped  logical_bytes  on-disk_bytes
wide                        68      801          201          16           1        0        2560000         3276800
deep                        59       35           33           2           1        1           4096            4096
mixed_size                  57        8            2           2           1        0        3163827         3174400
mixed_age                  174        7            3           0           2        0          16384           16384
overlapping_roots           58        6            3           2           1        0          12288           12288
inaccessible                97        3            2           2           1        1           4096            4096   (unix only)
symlink                     96        4            2           2           1        0           4155            4096   (unix only)
cancellation                60        2            1           1           1        0           4096            4096   (cancelled)
```

Windows CI regenerates the portable fixture baseline and runs the native
junction tests. This is Windows runner coverage, not a supported Windows 11
desktop baseline. The former numeric Windows table omitted its exact OS build
and run link, so it is no longer presented as reproducible machine evidence.
The remaining desktop checks are tracked in [WINDOWS_VALIDATION.md](WINDOWS_VALIDATION.md).

Two byte populations are reported, and only one of them is a committed fact:
`logical_bytes` is a property of the tree and is identical on every machine,
while the on-disk population is the platform's answer (APFS rounds each file up
to a block; `GetCompressedFileSizeW` reports what a file actually occupies, and
a small resident file is reported at its logical size). The benchmark commits
the logical population and bounds the on-disk one per fixture.

The rows also carry `rss_growth_kib`: this process's approximate resident-set
growth across the scan. It is reported and never asserted — the allocator
decides when pages return to the operating system, so two identical runs
disagree, and most of the growth belongs to building the fixture rather than to
walking it.

The `wide` row reaches `peak_tasks` 16, which is the bound being exercised
rather than trivially satisfied; the `cancellation` row shows the work actually
stopped (one directory read, no later candidate) and the result states
`cancelled: true`.

## Windows-specific behaviour

Reparse points, junctions, locked files, and sparse/compressed accounting are
stated in [WINDOWS_VALIDATION.md](WINDOWS_VALIDATION.md); the traversal rule is
that the walk uses one classifier (`SymlinkGuard`) for links and reparse points
in every walk, so a junction is a boundary in the walker and in the size
measurement alike rather than only in one of them.

## Repeating a read-only real-machine scan

Run the synthetic suite above first, then the explicit live observation tool:

```sh
cargo run -p zenith-desktop --example scan_machine -- --live-read-only

# Add fixed-argument tool-provider and container inspection. These modes still
# construct no cleanup plan and invoke no prune/delete operation.
cargo run -p zenith-desktop --example scan_machine -- \
  --live-read-only --providers-read-only --containers-read-only

# Stop provider measurement immediately after its first root progress event.
cargo run -p zenith-desktop --example scan_machine -- \
  --live-read-only --providers-read-only --providers-cancel-after-first-root
```

The tool scans a fixed subset of the shipped filesystem catalog with intensive
observation enabled. Provider inspection is opt-in and runs only the reviewed
cache-directory discovery commands before measuring the approved roots;
container inspection is opt-in and runs fixed status/list commands plus local
OrbStack metadata. It constructs no cleanup plan and invokes no cleanup
executor, provider prune, container prune, delete, or Trash operation. A
cooperative 60-second deadline returns a cancelled/partial result; it is not an
OS-level timeout for a stalled filesystem call. Output contains catalog IDs,
typed gaps, and aggregate counts/bytes, never item paths, user names, tool
output, free-form errors, or container identifiers. Record the OS build,
hardware, date, and version alongside the JSON. A zero-item signature is
missing coverage, not a passing application-specific check.

The [0.3.45 Mac record](validation/2026-09-21-macos-0.3.45.json) contains both
synthetic fixture results and an actual cache scan. The live result was partial:
sandbox access refusals remain visible in the incomplete/skipped totals. No
cleanup ran. Cursor/Go produced no items, so that run does not establish their
real-machine behavior. Timings include background developer activity and are
observations for comparison, not new pass/fail thresholds. Windows 11 desktop
validation and the uncovered Mac application/provider cases remain open in
#224/#227; #221 cannot close on this evidence alone.

The [0.3.48 Mac record](validation/2026-09-22-macos-0.3.48.json) adds pnpm/npm
cache measurements, provider progress and cancellation, a typed uv inspection
failure, Docker-daemon availability, and OrbStack aggregate metadata. The
filesystem observation is still partial, Docker was not running, and
Cursor/Go still produced no items. The evidence classes and remaining closure
boundary are summarized in [VALIDATION_SIGNOFF.md](VALIDATION_SIGNOFF.md).
