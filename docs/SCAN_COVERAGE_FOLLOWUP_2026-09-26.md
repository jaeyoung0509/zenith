# Scan coverage follow-up — 2026-09-26

The read-only diagnostic now groups incomplete observations by the catalog
owner, source ID, coarse root class, and typed reason. It emits bounded wall
time spans for each filesystem signature and owner provider. The default
report contains no paths; the deliberately separate `--private-ledger` option
remains the only path-level diagnostic. Partial or inaccessible bytes stay
unknown and outside observed, cleanable, and selected totals.

## Read-only live scan

On the audited Mac (Zenith 0.3.60, full embedded catalog, intensive scan,
normal user, no cleanup), one warm run before the DotSlash adapter reported:

| Measurement | Result |
| --- | ---: |
| Engine time / process wall time | 8,595 ms / 9.35 s |
| Peak process resident size | 120,586,240 bytes |
| Visited entries / directories read | 12,265 / 2,201 |
| Incomplete items / skipped entries | 477 / 479 |
| Possible Full Disk Access / permission denied / I/O gaps | 452 / 19 / 6 |
| Chrome HTTP / code cache observed and eligible | 125,472,768 / 65,634,304 bytes |
| Chrome cache preselected | 0 bytes (Rebuild requires review) |

The two protected container signatures accounted for all 452 possible Full
Disk Access gaps: 310 in application containers and 142 in group containers.
Their individual signature spans were 1,103 ms and 1,567 ms. The package-cache
provider group took 731 ms. These spans are elapsed wall time, not CPU time,
and can overlap with worker activity inside a scan step. Neither a permission
refusal nor an unreadable root proves that the root is unsupported. The
catalog therefore retains both scoped probes; pruning either would silently
hide potentially measurable bytes after Full Disk Access is granted. A future
prune needs evidence that an exact path is unsupported on a stated macOS
version, not merely blocked on this machine.

Brave had no installed cache candidate in this live scan. Fixture coverage
exercises two Chrome and two Brave profiles, recent generated cache content,
protected profile state, and a symlinked cache root. Fresh process checks now
mark a verified cache unavailable when owner state cannot be read, and a
running owner remains visible without being preselected. Planning rechecks
the owner before authorization; execution retains its own final owner and
identity checks. No Service Worker, cookie, history, or on-device model path
was added to the cleanup catalog.

A later read-only warm run including the DotSlash adapter took 9,513 ms in the
engine, visited 12,509 entries, read 2,206 directories, and retained 477
incomplete items with 479 skipped entries. Its typed gaps were 452 possible
Full Disk Access, 19 permission denied, and 6 I/O errors. The adapter observed
537,231,360 bytes across two complete DotSlash objects, with zero bytes
eligible under the 30-day rule on this machine. Their combined provider span
was 47 ms. Recent or structurally refused objects remain observed without
being mislabeled as I/O failures. No live cache was removed.

## Controlled fixture

`scan_metrics_match_the_committed_baseline` generated temporary trees and
scanned only those roots. On this Mac, its wide fixture completed in 80 ms,
visited 801 entries, read 201 directories, peaked at 16 outstanding directory
tasks, and had approximately 4,144 KiB resident growth. The deep fixture
completed in 162 ms, visited 35 entries, read 33 directories, and reported
one skipped entry at the depth bound. The test compares deterministic counts
with the committed baseline; elapsed time and resident growth are observations
from this run only.

The reference cleaner source informed the review of owner boundaries, live
process checks, and partial coverage. Zenith's catalog, authorization, and
reporting code were written independently. No cleanup ran against user data.
