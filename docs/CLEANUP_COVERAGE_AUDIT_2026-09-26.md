# macOS cleanup coverage audit — 2026-09-26

This is a read-only comparison of the reference cleaner and Zenith on one Mac. It identifies
authorization, measurement, and coverage differences; it is not a target for
Zenith to match the reference cleaner's headline number. Private path ledgers and command output
were kept outside the repository. No cleanup was performed on user data; a
disposable temporary fixture was cleaned to profile the executor.
The user reports that the reference cleaner's estimates have often been close to later actual
cleanup, but no paired deletion run was available for this snapshot.

## Reproduction boundary

| Fact | Value |
| --- | --- |
| Host | macOS 27.0 build 26A428, arm64 |
| Reference cleaner | CLI 1.55.0, read-only source snapshot at tag `V1.55.0` (`69ab325`) |
| Zenith | 0.3.60, embedded catalog plus native lifecycle/owner providers; intensive cleanup enabled; no exclusions |
| Privilege | Normal user; no sudo or system-cache preview |
| Reference cleaner command | `mo clean --dry-run`, then read its private preview list |
| Zenith command | `scan_machine --live-read-only --full-catalog-read-only --private-ledger` |
| Owner previews | `HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_AUTOREMOVE=1 brew cleanup --dry-run --prune=7` and `--prune=30` |
| Run order / cache state | Fixed-signature diagnostic first; full Zenith scan at 05:45 UTC; the reference cleaner preview around 06:05 UTC; full Zenith scan at 06:06 UTC; later Homebrew 7/30 dry-runs and warm post-change Zenith scan. No cache reset was performed, so these are warm/uncontrolled filesystem observations rather than a cold/warm experiment. |

The full Zenith diagnostic uses the same embedded catalog, native provider
registries, and `ScanEngine` as the desktop composition root. It does not load
user-replaced catalogs or model window preferences. A UI scan with different
settings can therefore differ. The earlier fixed-signature diagnostic from the
issue is narrower and is **not** the denominator below. That restricted run
observed 998,993,920 bytes, marked 465 items incomplete, visited 8,668
entries in 6.78 s, and selected 251,801,600 bytes; it used no owner or
lifecycle providers.

For a local reproduction, set `reference_list_path` to the CLI's private
preview-list path and use private files with owner-only permissions:

```sh
umask 077
mo clean --dry-run > /tmp/reference-clean-dry-run.txt 2>&1
reference_list_path=/absolute/path/to/clean-list.txt
cargo run -q -p zenith-desktop --example scan_machine -- \
  --live-read-only --full-catalog-read-only --private-ledger \
  > /tmp/zenith-full-private.json
python3 scripts/cleanup_coverage_audit.py \
  --reference-preview "$reference_list_path" \
  --zenith-report /tmp/zenith-full-private.json \
  --measure-allocated > /tmp/coverage-summary.json
```

The script's default output contains aggregate counts and no paths. Its
`--private-details` option deliberately includes paths and must stay local.
`--measure-allocated` calls bounded `du -skP` on preview candidates and records
failure as unknown, never zero. It measures current allocated footprint; it
does not predict what an owner command would delete. The reference cleaner list contains
rounded display sizes and nested paths, so its displayed total is not a
deduplicated filesystem union or a verified disk-free delta.

## Measured baseline

| Read-only observation | Result |
| --- | ---: |
| Reference cleaner preview at ~06:05 UTC | 2.85 GB potential; 75 listed items, 5 sections |
| Reference cleaner listed paths that sit inside another listed path | 24; rounded displayed sizes sum to ~1.15 GB |
| Reference cleaner rounded display sum, excluding structurally nested paths | ~1.86 GB; **still not a reclaim estimate** |
| Zenith full scan at 06:06 UTC, before catalog change | 3,084,984,320 observed bytes; 653,668,352 cleanable; 623,190,016 preselected |
| Zenith scan quality | Partial; 477 incomplete items and 479 skipped entries |
| Typed gap counts | 452 possible Full Disk Access, 19 permission denied, 6 I/O error |
| Zenith traversal | 10,399 entries, 2,126 directories, 8.26 s, peak 16 outstanding tasks |
| Comparison-to-Zenith path relation | 13 exact, 44 within a Zenith unit, 18 comparison-only; a parent relation does **not** establish equal cleanup semantics |

The environment changed during the audit. A full Zenith scan at 05:45 UTC
observed 651,608,064 bytes under Homebrew; at 06:06 UTC the same root observed
1,023,700,992 bytes. The corresponding whole-scan observed totals changed
from 2,263,781,376 to 3,084,984,320 bytes. The source of this intervening
growth was not established. It demonstrates why measurements made twenty
minutes apart must not be treated as a controlled A/B result. The paired reference-cleaner
preview and 06:06 Zenith scan are close in time, but their scope and policy
still differ.

The explicit Homebrew inventory rule introduced by this audit moves its bytes
from generic automatic application cleanup into Manual inventory. A follow-up
scan observed 1,023,700,992 Homebrew bytes, with **zero generic cleanable
bytes**. Overall cleanable bytes fell to 30,478,336 and preselected bytes to
zero. That reduction is intentional: a preview of a tool-owned cache cannot
authorize generic deletion. The observed total remains about 3.09 GB. These
figures are not evidence of a smaller physical cache.

The subsequent implementation in this PR replaced that interim Homebrew Manual
root with a reviewed provider for direct downloaded files and kept its API and
bootsnap metadata advisory. It also added exact Chrome HTTP/code cache and
Help Viewer generated/page cache units. A later read-only full scan observed
3,190,308,864 bytes overall, of which 1,240,965,120 bytes were cleanable and
zero were preselected. The new reviewed units accounted for 968,634,368 bytes
of Homebrew downloads, 137,175,040 bytes of Chrome cache, and 29,720,576
bytes of Help Viewer cache. These are `Rebuild` units: the user must select and
review them, and future installations/pages may re-download or regenerate
data. This later scan is not a controlled subtraction from the 06:06 baseline.

## Path and owner ledger

| Owner / candidate | Reference cleaner evidence | Zenith evidence and reason | Decision |
| --- | --- | --- | --- |
| Homebrew | Root and 20 nested download paths in the reference cleaner's list; root footprint ~1.02 GB. Its broad user-cache sweep passes the Homebrew root to its guarded remover, independently of its separate `brew cleanup` step. | The interim Manual root prevented generic deletion; the final owner provider offers 968,634,368 bytes of direct download files for explicit Rebuild review. API/bootsnap and unrecognized entries stay advisory. | Homebrew's **own** `brew cleanup --dry-run --prune=7` and `--prune=30` produced byte-identical output: 10 candidate lines and ~164.2 MB for this snapshot. Those lines named old Cellar versions, temporary Cellar staging, and empty prefix directories, not cache downloads. This 164.2 MB is **not** an upper bound on what the reference cleaner's broader cache removal may reclaim. |
| dotslash | One comparison-only candidate displayed ~537 MB | Explicitly excluded from Zenith's broad cache rule. A trial generic Manual inventory could not completely measure its protected contents and produced a zero-byte partial row, so it was not retained. | A follow-up owner-scoped adapter now offers only complete hash-addressed artifacts unchanged for 30 days, after an opt-in and individual Rebuild review. The historical ~537 MB whole-root preview is not its eligible amount. |
| Chrome cache | Reference cleaner listed a ~127.6 MB `~/Library/Caches/Google/Chrome/Default` candidate | Zenith originally observed its `Google` parent as recent and selected zero. Exact HTTP and code cache units now offer 137,175,040 bytes in the later scan for review. | Require Chrome and helpers to be stopped. Keep `Storage`, profile state, cookies, Service Workers, and on-device models outside these units. |
| Help Viewer cache | Reference cleaner listed generated and cached page paths totaling ~29.8 MB | Exact generated and page cache units now offer 29,720,576 bytes in the later scan. | Require `helpd` to be stopped; retain HSTS, preferences, and neighboring indexes. |
| Cargo registry archive | Reference cleaner listed ~30.5 MB | Zenith owner provider measured 30,478,336 bytes cleanable, but Rebuild requires selection, so zero preselected. | Already covered; explain selection rather than add another path rule. |
| Other small comparison-only paths | 17 other listed paths after Homebrew becomes an exact advisory match; several are Apple/tool namespaces or shell artifacts | Many are intentionally excluded or outside Zenith's current catalog. | Triage individually by owner; small generic paths do not justify a broad deletion rule. |
| Protected macOS containers and app data | Not the main source of the reference cleaner's Homebrew/cache preview difference | 452 possible Full Disk Access gaps were largely sandbox/group-container cache probes; unavailable units retained zero observed bytes with typed partial quality. | Improve visibility of the reason and assess whether protected Apple roots should be attempted at all. Never convert these zeros to complete observations. |

The reference cleaner's preview is not the same as `brew cleanup`'s preview. Its pinned
`V1.55.0` general user-cache sweep passes direct children of
`~/Library/Caches`, including `Homebrew`, to its guarded removal path. Its
separate Homebrew dry-run branch prints “would cleanup” without running
`brew cleanup --dry-run` or applying its run cooldown/size threshold. Thus
the reference cleaner may actually remove more cache data than `brew cleanup` would select.
Changing Homebrew's prune age from 30 to 7 days made no difference in this
paired dry-run: the output hashes and candidate lines were identical. The
remaining discrepancy is therefore not explained by those age settings on
this snapshot. Homebrew's [cleanup contract](https://docs.brew.sh/Manpage#cleanup-options-formula-cask-)
includes old installed formula versions as well as cache entries. A dry-run
predicts an operation but does not prove a later disk-free delta.

A later read-only inspection found 945,932 KiB in `Homebrew/downloads`,
30,576 KiB in `api`, and 12,564 KiB in `bootsnap`. Among direct regular files
in `downloads`, allocated bytes were ~356 MB under 7 days old, ~475 MB at
7–30 days, and ~137 MB older than 30 days. These are a later, changing
snapshot, not a second paired baseline. They show why an age-gated or
Homebrew-managed cleanup can select much less than a deliberate full-download
cache purge. The user's prior runs with the reference cleaner often reclaimed close to its estimate;
that is consistent with the generic sweep, though no controlled actual
cleanup was run for this audit.
The source-level distinction is documented in [issue #294](https://github.com/jaeyoung0509/zenith/issues/294).

## Accounting and throughput limits

| Quantity | This audit |
| --- | --- |
| Observed / cleanable / selected | Raw Zenith bytes in the table above; after the Homebrew classification change, cleanable/selected changed as stated |
| Reference cleaner potential | Rounded candidate display sizes, including nested entries |
| Actually removed from live user caches | Not measured; no cleanup ran on user data |
| Verified live reclaimed and disk-free delta | Not measured; no cleanup ran on user data |
| Scan wall time | Two read-only full runs: 10.46 s and 8.26 s; caches and filesystem content changed between them |
| Scan memory | A warm full read-only run through `/usr/bin/time -l` took 8.61 s wall time and peaked at 120,569,856 bytes RSS (~115 MiB); the same scan reported 8.14 s inside `ScanEngine` |
| Per-owner scan latency | Not separately timed by the current scan event/metrics contract. Whole-scan duration and per-signature counts are available, but assigning shared parallel traversal time to a particular owner would be misleading; add timed spans before claiming an owner bottleneck. |
| Disposable executor fixture | 64 generated files, 4,194,304 logical and allocated bytes; 4,194,304 bytes reported reclaimed, zero files remaining; executor elapsed 851 ms in one macOS run |

No Rust traversal slowdown is established by these runs. The largest observed
numerical gaps are selection/authority and accounting, not proven scan speed.
The disposable benchmark uses the ordinary scan, plan, validation, and
executor path. Its file count and allocation are fixture facts; its 851 ms is
a single local observation, not a performance guarantee. Running
the reference cleaner and Zenith sequentially on the user's live caches would change the second
tool's input and would not be a fair throughput comparison.

## Ranked implementation follow-ups

1. **[Homebrew reviewed action](https://github.com/jaeyoung0509/zenith/issues/295):** the
   direct-download provider is implemented here. Remaining work is a separate
   `brew cleanup` operation, a grouped preview for many download files, and a
   controlled post-action disk-free measurement. The
   [operation decision](HOMEBREW_CLEANUP_DECISION.md) explains why the narrower
   command is not offered yet. Keep it out of Quick Panel Safe cleanup.
2. **[Browser cache review](https://github.com/jaeyoung0509/zenith/issues/296):** exact Chrome
   HTTP/code cache units and a process guard are implemented here. Continue
   profile and browser-family coverage without adding offline/profile state.
3. **[dotslash owner contract](https://github.com/jaeyoung0509/zenith/issues/297):**
   the [coverage decision](DOTSLASH_CACHE_DECISION.md) records the selective
   owner-scoped adapter, its opt-in review, and why unreadable bytes remain unknown.
4. **[Partial scan quality](https://github.com/jaeyoung0509/zenith/issues/298):** count protected-root refusals by typed cause and
   present unavailable bytes as unknown, not zero usable capacity. Assess
   bounded selector pruning without weakening the scan/plan/execution guard.
   The [follow-up measurement](SCAN_COVERAGE_FOLLOWUP_2026-09-26.md) records
   typed groups, scan-step timings, and the decision to retain protected probes.
5. **[Optimize boundary](https://github.com/jaeyoung0509/zenith/issues/299):**
   the [maintenance decision record](MACOS_MAINTENANCE_DECISIONS.md) evaluates
   each action's preview, consent, privilege, and verification boundary. None
   is a cleanup-byte target in the current user-space cleaner.

### Optimize action inventory

The pinned reference-cleaner catalog has 21 actions. Its dry-run reported 3 that would
apply, 14 unchanged, 1 skipped, 1 unavailable, and 2 failed on this Mac.
Those counts are outcomes of a maintenance preview, **not bytes**. The table
below groups actions by the Zenith capability and verification they would
require; it does not grant execution permission.

| Reference cleaner action IDs | Zenith capability/preview and consent boundary | Post-action check |
| --- | --- | --- |
| `system_maintenance`, `network_optimization`, `network_stack_optimize` | Separate DNS, Spotlight-status, mDNSResponder, route, and ARP actions; show the exact service or table affected, required privilege, and a typed unavailable/skip reason. Explicit consent before mutation. | Query the same service or network state; report any failed sub-action. |
| `cache_refresh`, `launch_services_rebuild`, `notification_cleanup` | Preview the Finder/QuickLook, LaunchServices, or notification store to be rebuilt or cleared. These have different owners and cannot share a generic cache cleanup command. | Re-query each service and report a bounded success/failure result. |
| `saved_state_cleanup`, `fix_broken_configs`, `sqlite_vacuum`, `shared_file_list_repair`, `quarantine_cleanup`, `coreduet_cleanup` | Enumerate exact stateful files and owners, require an app-closed guard, backup/rollback plan where applicable, and explicit consent. Several alter personal or security history; no automatic action follows from a dry-run count. | Re-open or validate the affected store and compare pre/post state, never just command exit status. |
| `prevent_network_dsstore`, `legacy_overrides_audit`, `spotlight_orphan_rules_cleanup`, `login_items_audit`, `launch_agents_cleanup` | Preview each preference, rule, or registration with current value and proposed change; protect managed settings and require a per-item decision. | Read the preference or registration back and report unchanged/skipped separately. |
| `disk_permissions_repair`, `spotlight_index_optimize`, `periodic_maintenance`, `disk_verify` | Treat each as a separate privileged or long-running system capability, with impact, expected duration, cancel/skip behavior, and a dry-run that names the exact operation. No implicit sudo. | Re-run the relevant permissions, index, periodic-script, or filesystem health check. |

Zenith currently has no corresponding maintenance workflow. A follow-up
should first decide which actions solve a user problem and can be safely
previewed on supported macOS versions; it should not copy the reference cleaner's catalog or
its automatic-execution classification.

The reference cleaner's source is GPL-3.0 and Zenith is MIT. This audit records observed behavior
and references pinned source; no implementation, test, table, or text from that source was
copied into Zenith.
