# macOS maintenance decisions

This records the decision for the 21 actions observed in the read-only
maintenance preview in [the cleanup coverage audit](CLEANUP_COVERAGE_AUDIT_2026-09-26.md).
The preview reported three actions that would apply, fourteen unchanged, one
skipped, one unavailable, and two failed. None reported reclaimable bytes.

## Decision rule

An action can enter Zenith only after a platform adapter can name the exact
object and proposed change before execution, state whether elevation or a
restart is needed, obtain explicit consent, bound and cancel its work, and
verify the same object afterwards. A mutation of personal or structured state
also needs a backup and rollback design. A dry-run status alone grants no
authority. These actions never enter generic Cleanup or its byte totals.

| Preview action | Decision and missing contract | Preview and verification required before implementation |
| --- | --- | --- |
| `system_maintenance` | Defer: combines unrelated OS work. | Split into named service actions; verify each independently. |
| `network_optimization` | Defer: network state may be managed. | Show exact interface and state; compare after action. |
| `network_stack_optimize` | Defer: route and resolver changes can interrupt connectivity. | Show exact tables and planned changes; verify restoration. |
| `cache_refresh` | Defer: cache owners differ. | Name each cache owner and regeneration cost; re-query its state. |
| `launch_services_rebuild` | Defer: system registration is shared state. | Show affected registration database and restart impact; verify registrations. |
| `notification_cleanup` | Defer: may alter personal notification history. | Inventory exact records and provide backup/rollback. |
| `saved_state_cleanup` | Defer: saved app sessions are personal state. | Identify app and files, require the app closed, then verify a restorable backup. |
| `fix_broken_configs` | Defer: generic repair has no bounded target. | Show exact configuration diff and restore path. |
| `sqlite_vacuum` | Defer: live databases and WAL files need owner coordination. | Identify database and owner, back it up, then run integrity checks. |
| `shared_file_list_repair` | Defer: recent items and favorites are personal state. | Show exact entry changes and a reversible export. |
| `quarantine_cleanup` | Reject as a generic action: quarantine is a security boundary. | A future per-file workflow would need a reason and explicit decision. |
| `coreduet_cleanup` | Reject as a generic action: OS-owned behavioral history. | No automatic or generic mutation contract. |
| `prevent_network_dsstore` | Defer: changes a user preference. | Show current and proposed value; read it back after consent. |
| `legacy_overrides_audit` | Keep read-only: an audit does not imply mutation. | Report exact setting and owner without applying a fix. |
| `spotlight_orphan_rules_cleanup` | Defer: indexing rules can be managed. | Enumerate exact orphan evidence and preserve previous values. |
| `login_items_audit` | Keep read-only: login items are user choices. | Report registration and owner; any removal needs per-item consent. |
| `launch_agents_cleanup` | Defer: agents may run user services. | Show exact plist, process and owner; back up before removal. |
| `disk_permissions_repair` | Reject as a generic action: privilege and scope are unclear. | A future action needs an OS-supported exact target and no implicit elevation. |
| `spotlight_index_optimize` | Defer: potentially long-running system work. | Show volume, expected duration and power impact; verify index status. |
| `periodic_maintenance` | Defer: opaque scripts have no exact preview. | Name each script and target before considering execution. |
| `disk_verify` | Keep as a separate read-only diagnostic candidate. | Show selected volume, duration and result; never count bytes reclaimed. |

No action currently passes that gate. Zenith therefore adds no maintenance
mutation or privilege request in this change. A future issue should be scoped
to one named action only after its preview, cancellation, and post-action
contract have been demonstrated on a supported macOS version. The upstream
GPL source was used for behavior comparison only; no implementation or catalog
table was copied.
