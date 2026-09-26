# macOS cleanup coverage audit — 2026-09-26

This is a read-only comparison of Mole CLI and Zenith on one Mac. It identifies
authorization, measurement, and coverage differences; it is not a target for
Zenith to match Mole's headline number. Private path ledgers and command output
were kept outside the repository. No cleanup was performed on user data; a
disposable temporary fixture was cleaned to profile the executor.
The user reports that Mole's estimates have often been close to later actual
cleanup, but no paired deletion run was available for this snapshot.

## Reproduction boundary

| Fact | Value |
| --- | --- |
| Host | macOS 27.0 build 26A428, arm64 |
| Mole | CLI 1.55.0, source tag `V1.55.0` at [`69ab325`](https://github.com/tw93/Mole/tree/69ab325d4f05af0ea21aeeeae544046c9f04a76b) |
| Zenith | 0.3.60, embedded catalog plus native lifecycle/owner providers; intensive cleanup enabled; no exclusions |
| Privilege | Normal user; no sudo or system-cache preview |
| Mole command | `mo clean --dry-run`, then read its private `~/.config/mole/clean-list.txt` |
| Zenith command | `scan_machine --live-read-only --full-catalog-read-only --private-ledger` |
| Owner previews | `HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_AUTOREMOVE=1 brew cleanup --dry-run --prune=7` and `--prune=30` |
| Run order / cache state | Fixed-signature diagnostic first; full Zenith scan at 05:45 UTC; Mole preview around 06:05 UTC; full Zenith scan at 06:06 UTC; later Homebrew 7/30 dry-runs and warm post-change Zenith scan. No cache reset was performed, so these are warm/uncontrolled filesystem observations rather than a cold/warm experiment. |

The full Zenith diagnostic uses the same embedded catalog, native provider
registries, and `ScanEngine` as the desktop composition root. It does not load
user-replaced catalogs or model window preferences. A UI scan with different
settings can therefore differ. The earlier fixed-signature diagnostic from the
issue is narrower and is **not** the denominator below. That restricted run
observed 998,993,920 bytes, marked 465 items incomplete, visited 8,668
entries in 6.78 s, and selected 251,801,600 bytes; it used no owner or
lifecycle providers.

For a local reproduction, use private files with owner-only permissions:

```sh
umask 077
mo clean --dry-run > /tmp/mole-clean-dry-run.txt 2>&1
cp "$HOME/.config/mole/clean-list.txt" /tmp/mole-clean-list.txt
cargo run -q -p zenith-desktop --example scan_machine -- \
  --live-read-only --full-catalog-read-only --private-ledger \
  > /tmp/zenith-full-private.json
python3 scripts/cleanup_coverage_audit.py \
  --mole-preview /tmp/mole-clean-list.txt \
  --zenith-report /tmp/zenith-full-private.json \
  --measure-allocated > /tmp/coverage-summary.json
```

The script's default output contains aggregate counts and no paths. Its
`--private-details` option deliberately includes paths and must stay local.
`--measure-allocated` calls bounded `du -skP` on preview candidates and records
failure as unknown, never zero. It measures current allocated footprint; it
does not predict what an owner command would delete. The Mole list contains
rounded display sizes and nested paths, so its displayed total is not a
deduplicated filesystem union or a verified disk-free delta.

## Measured baseline

| Read-only observation | Result |
| --- | ---: |
| Mole preview at ~06:05 UTC | 2.85 GB potential; 75 listed items, 5 sections |
| Mole listed paths that sit inside another listed path | 24; rounded displayed sizes sum to ~1.15 GB |
| Mole rounded display sum, excluding structurally nested paths | ~1.86 GB; **still not a reclaim estimate** |
| Zenith full scan at 06:06 UTC, before catalog change | 3,084,984,320 observed bytes; 653,668,352 cleanable; 623,190,016 preselected |
| Zenith scan quality | Partial; 477 incomplete items and 479 skipped entries |
| Typed gap counts | 452 possible Full Disk Access, 19 permission denied, 6 I/O error |
| Zenith traversal | 10,399 entries, 2,126 directories, 8.26 s, peak 16 outstanding tasks |
| Mole-to-Zenith path relation | 13 exact, 44 within a Zenith unit, 18 Mole-only; a parent relation does **not** establish equal cleanup semantics |

The environment changed during the audit. A full Zenith scan at 05:45 UTC
observed 651,608,064 bytes under Homebrew; at 06:06 UTC the same root observed
1,023,700,992 bytes. The corresponding whole-scan observed totals changed
from 2,263,781,376 to 3,084,984,320 bytes. The source of this intervening
growth was not established. It demonstrates why measurements made twenty
minutes apart must not be treated as a controlled A/B result. The paired Mole
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

| Owner / candidate | Mole evidence | Zenith evidence and reason | Decision |
| --- | --- | --- | --- |
| Homebrew | Root and 20 nested download paths in the Mole list; root footprint ~1.02 GB. Mole's broad user-cache sweep passes the Homebrew root to its guarded remover, independently of its separate `brew cleanup` step. | The interim Manual root prevented generic deletion; the final owner provider offers 968,634,368 bytes of direct download files for explicit Rebuild review. API/bootsnap and unrecognized entries stay advisory. | Homebrew's **own** `brew cleanup --dry-run --prune=7` and `--prune=30` produced byte-identical output: 10 candidate lines and ~164.2 MB for this snapshot. Those lines named old Cellar versions, temporary Cellar staging, and empty prefix directories, not cache downloads. This 164.2 MB is **not** an upper bound on what Mole's broader cache removal may reclaim. |
| dotslash | One Mole-only candidate displayed ~537 MB | Explicitly excluded from Zenith's broad cache rule. A trial generic Manual inventory could not completely measure its protected contents and produced a zero-byte partial row, so it was not retained. | Keep excluded; research an owner-supported inventory and invalidation contract. Do not relabel this entire root cleanable. |
| Chrome cache | Mole listed a ~127.6 MB `~/Library/Caches/Google/Chrome/Default` candidate | Zenith originally observed its `Google` parent as recent and selected zero. Exact HTTP and code cache units now offer 137,175,040 bytes in the later scan for review. | Require Chrome and helpers to be stopped. Keep `Storage`, profile state, cookies, Service Workers, and on-device models outside these units. |
| Help Viewer cache | Mole listed generated and cached page paths totaling ~29.8 MB | Exact generated and page cache units now offer 29,720,576 bytes in the later scan. | Require `helpd` to be stopped; retain HSTS, preferences, and neighboring indexes. |
| Cargo registry archive | Mole listed ~30.5 MB | Zenith owner provider measured 30,478,336 bytes cleanable, but Rebuild requires selection, so zero preselected. | Already covered; explain selection rather than add another path rule. |
| Other small Mole-only paths | 17 other listed paths after Homebrew becomes an exact advisory match; several are Apple/tool namespaces or shell artifacts | Many are intentionally excluded or outside Zenith's current catalog. | Triage individually by owner; small generic paths do not justify a broad deletion rule. |
| Protected macOS containers and app data | Not the main source of Mole's Homebrew/cache preview difference | 452 possible Full Disk Access gaps were largely sandbox/group-container cache probes; unavailable units retained zero observed bytes with typed partial quality. | Improve visibility of the reason and assess whether protected Apple roots should be attempted at all. Never convert these zeros to complete observations. |

Mole's preview is not the same as `brew cleanup`'s preview. Mole's pinned
`V1.55.0` general user-cache sweep passes direct children of
`~/Library/Caches`, including `Homebrew`, to its guarded removal path. Its
separate Homebrew dry-run branch prints “would cleanup” without running
`brew cleanup --dry-run` or applying its run cooldown/size threshold. Thus
Mole may actually remove more cache data than `brew cleanup` would select.
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
cache purge. The user's prior Mole runs often reclaimed close to its estimate;
that is consistent with the generic sweep, though no controlled actual
cleanup was run for this audit.
The source-level distinction is documented in [issue #294](https://github.com/jaeyoung0509/zenith/issues/294).

## Accounting and throughput limits

| Quantity | This audit |
| --- | --- |
| Observed / cleanable / selected | Raw Zenith bytes in the table above; after the Homebrew classification change, cleanable/selected changed as stated |
| Mole potential | Rounded candidate display sizes, including nested entries |
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
Mole and Zenith sequentially on the user's live caches would change the second
tool's input and would not be a fair throughput comparison.

## Ranked implementation follow-ups

1. **[Homebrew reviewed action](https://github.com/jaeyoung0509/zenith/issues/295):** the
   direct-download provider is implemented here. Remaining work is a separate
   `brew cleanup` operation, a grouped preview for many download files, and a
   controlled post-action disk-free measurement. Keep it out of Quick Panel
   Safe cleanup.
2. **[Browser cache review](https://github.com/jaeyoung0509/zenith/issues/296):** exact Chrome
   HTTP/code cache units and a process guard are implemented here. Continue
   profile and browser-family coverage without adding offline/profile state.
3. **[dotslash owner contract](https://github.com/jaeyoung0509/zenith/issues/297):** find a supported way to enumerate inactive
   objects and measure protected bundles completely; otherwise keep advisory.
4. **[Partial scan quality](https://github.com/jaeyoung0509/zenith/issues/298):** count protected-root refusals by typed cause and
   present unavailable bytes as unknown, not zero usable capacity. Assess
   bounded selector pruning without weakening the scan/plan/execution guard.
5. **[Optimize boundary](https://github.com/jaeyoung0509/zenith/issues/299):** keep system maintenance separate. Mole's catalog
   includes DNS, Finder, Spotlight, LaunchServices, database, network, and
   login-item operations; each needs its own platform capability, dry-run,
   permission model, reason for skip/failure, and post-action check. None is a
   cleanup-byte target in the current user-space cleaner.

### Optimize action inventory

The pinned Mole catalog has 21 actions. Its dry-run reported 3 that would
apply, 14 unchanged, 1 skipped, 1 unavailable, and 2 failed on this Mac.
Those counts are outcomes of a maintenance preview, **not bytes**. The table
below groups actions by the Zenith capability and verification they would
require; it does not grant execution permission.

| Mole action IDs | Zenith capability/preview and consent boundary | Post-action check |
| --- | --- | --- |
| `system_maintenance`, `network_optimization`, `network_stack_optimize` | Separate DNS, Spotlight-status, mDNSResponder, route, and ARP actions; show the exact service or table affected, required privilege, and a typed unavailable/skip reason. Explicit consent before mutation. | Query the same service or network state; report any failed sub-action. |
| `cache_refresh`, `launch_services_rebuild`, `notification_cleanup` | Preview the Finder/QuickLook, LaunchServices, or notification store to be rebuilt or cleared. These have different owners and cannot share a generic cache cleanup command. | Re-query each service and report a bounded success/failure result. |
| `saved_state_cleanup`, `fix_broken_configs`, `sqlite_vacuum`, `shared_file_list_repair`, `quarantine_cleanup`, `coreduet_cleanup` | Enumerate exact stateful files and owners, require an app-closed guard, backup/rollback plan where applicable, and explicit consent. Several alter personal or security history; no automatic action follows from a dry-run count. | Re-open or validate the affected store and compare pre/post state, never just command exit status. |
| `prevent_network_dsstore`, `legacy_overrides_audit`, `spotlight_orphan_rules_cleanup`, `login_items_audit`, `launch_agents_cleanup` | Preview each preference, rule, or registration with current value and proposed change; protect managed settings and require a per-item decision. | Read the preference or registration back and report unchanged/skipped separately. |
| `disk_permissions_repair`, `spotlight_index_optimize`, `periodic_maintenance`, `disk_verify` | Treat each as a separate privileged or long-running system capability, with impact, expected duration, cancel/skip behavior, and a dry-run that names the exact operation. No implicit sudo. | Re-run the relevant permissions, index, periodic-script, or filesystem health check. |

Zenith currently has no corresponding maintenance workflow. A follow-up
should first decide which actions solve a user problem and can be safely
previewed on supported macOS versions; it should not copy Mole's catalog or
its automatic-execution classification.

Mole source is GPL-3.0 and Zenith is MIT. This audit records observed behavior
and references pinned source; no Mole implementation, test, table, or text was
copied into Zenith.
