# Windows folder and feature validation

This document separates what automated CI actually checks from what still
requires a human on a real Windows machine. The manual matrix below is a
**plan**: no row has been executed and recorded yet, so none of it is evidence.
An unexecuted matrix is a plan, not a validation result.

Issue #122 addresses profile, drive, known-folder, Unicode picker, and directory
identity defects found in v0.3.0. The old v0.2.0 screenshots describe the
separate verbatim-path and console regressions fixed in #118.

## What CI actually verifies

The `rust-windows` job in `.github/workflows/ci.yml` runs
`cargo test --manifest-path src-tauri/Cargo.toml` on `windows-latest`, so the
Windows-gated (`#[cfg(windows)]`) tests run against real Windows APIs and a real
NTFS temporary directory. Those tests assert:

- Real volume/file identity is captured for a delete target, and a zeroed or
  changed identity is rejected (`windows_safety::directory_handle_captures_real_volume_file_identity`,
  `zero_identity_is_never_accepted_as_verified`,
  `file_id_change_is_rejected_as_ownership_changed`).
- A real NTFS junction created with `mklink /J` is recognized as a reparse
  point and is removed without being traversed, leaving its target untouched
  (`reparse_point_is_never_traversed_during_cleanup`).
- Protected paths are rejected case-insensitively, and long verbatim (`\\?\`)
  paths still resolve.
- `.git` directory rejection holds for a Windows temporary path.

Cross-platform fixture tests run on every runner. They exercise
`PlatformEnvironment::simulated` and the pure path algebra, so they cover
conflicting `HOME`/`USERPROFILE`, a non-`C:` system drive, a Korean user name,
a redirected known folder, a UNC profile, and volume records without a stable
identifier — as *simulated* inputs, not as a real machine's configuration:

- `windows_profile_does_not_follow_another_accounts_shell_home`
- `windows_profile_preserves_non_c_drive_and_korean_name`
- `a_redirected_windows_profile_resolves_placeholders_without_the_host`
- `redirected_known_folder_wins_over_the_profile_spelling`
- `unc_profile_is_representable`
- `simulated_profile_can_live_on_a_non_system_drive`
- `a_drive_rooted_fixture_protects_its_system_roots`

The packaging job also proves the per-user installer installs silently, passes
`Zenith --doctor`, uninstalls silently, and leaves no install directory, and it
proves the machine-wide installer packages. See
[`docs/WINDOWS.md`](WINDOWS.md).

## What CI does not verify

No automated check covers any of the following. Every one of them requires the
manual matrix below, run on a real machine:

- A real machine with a non-`C:` system drive.
- Real configured folder redirection (Known Folder Move, OneDrive, or
  administrator redirection), including a real UNC or domain-joined profile.
- Non-NTFS volumes, network drives, mounted VHDs, or filesystems without stable
  file identities under real use.
- Non-UTF-8 or non-English code pages and their effect on subprocess output.
- Standard-user (unprivileged) installation and uninstallation, and UAC
  behavior.
- Application-control policy: AppLocker default rules, WDAC, Smart App Control,
  or any managed-device restriction. CI runners have none of these enabled.
- A machine with the WebView2 runtime absent, installed through the embedded
  offline installer under interactive use.
- Interactive installer UI, Start Menu, tray, and account-switch behavior.

## Manual smoke matrix (plan)

Use disposable Windows 10/11 accounts and fixture data. Never use valuable files
for cleanup tests.

Record for every run: **application version**, **Windows build** (`winver`),
**run date**, account type, system drive, and whether user folders are
redirected. Omit secrets from logs. A row without those fields is not a
validation result.

| Scenario | Expected result |
| --- | --- |
| Two accounts, conflicting `HOME` / `USERPROFILE` | Only the active profile is the automatic scan root |
| Korean username and project name with spaces | Paths and selected executable names round-trip intact |
| System installed on D:, with C: enumerated first | The primary card reports D:; mounted volumes remain independent |
| Documents/Desktop moved to another drive or OneDrive | Ordinary files in the OS-configured folders are discoverable |
| Windows Videos folder | The `movies` selection scans Videos |
| App installations under relocated Program Files / LocalAppData | Inventory uses configured directories |
| Picker cancellation | Empty successful result |
| Picker process failure or malformed output | Visible error, not silent cancellation or a corrupted path |
| Junction to another account or volume | Not traversed or admitted as a cleanup root |
| Folder replaced after scanning | Identity mismatch prevents using the old selection |
| Rebuildable artifact under the active profile | Explicit selection is required; project files are preserved |
| Managed machine with AppLocker default rules | Per-user install is refused; machine-wide install is permitted |
| Machine without WebView2 | Installer completes from the embedded offline WebView2 package |
| Standard user, no elevation | Per-user install succeeds; machine-wide install prompts for elevation |

Run-record template to append after each execution:

```text
Date:              2026-00-00
Application:       0.0.0
Windows build:     10.0.22631 (winver)
Installer:         per-user | machine-wide
Account:           standard | administrator
System drive:      C: | other
Redirection:       none | Known Folder Move | OneDrive | administrator policy
Scenario:          <row from the matrix>
Result:            pass | fail (details, evidence)
```

## Account and path behavior as implemented

- Automatic cache and developer scans use the current process account's
  `USERPROFILE`. A competing Git/MSYS `HOME` is not another scan root. Missing or
  invalid profile paths fail closed instead of switching accounts.
- The system storage card uses `SystemDrive`. Mounted volumes still list all
  disks; listing a drive does not authorize scanning or cleaning the entire drive.
- Large Files uses Windows Known Folders for Downloads, Desktop, Documents, and
  Videos (the existing IPC token is `movies`), resolved through
  `SHGetKnownFolderPath`, so a configured folder redirection is honored. Only
  these fixed tokens are accepted; the frontend does not submit arbitrary paths.
- Developer workspace selection remains restricted to children of the current
  profile. Selecting an arbitrary `D:\projects` outside that profile is currently
  unsupported. This is a deliberate existing safety boundary, not drive discovery.
- Junctions, symbolic links, and other reparse points are skipped. Cloud-only
  OneDrive placeholders may therefore be absent; this change does not hydrate or
  traverse them. Use locally available ordinary files for the baseline test.
- Running as another user/admin selects that process account's profile. Zenith
  does not enumerate or clean all accounts on a shared PC.

## Completed domain adapters (#123)

Issue #123 completes the platform adapter boundaries identified during the initial Windows port. The statements below describe unit tests that run in CI;
they do not replace the manual matrix above for real-machine behavior.

1. **Keep Awake (`PowerAssertion`):**
   - Migrated from thread-local `SetThreadExecutionState` to owned `PowerCreateRequest`, `PowerSetRequest`, and `PowerClearRequest` handles.
   - Standard Rust `OwnedHandle` manages the process-scoped handle. RAII clears successfully enabled requests, including partial acquisition failures, before closing it on any worker thread.
   - Verified via unit tests covering acquisition, display/system request behavior changes, expiration, and multi-thread lifecycle.

2. **AI Activity (`agent_activity::adapters` & `agent_activity::mod`):**
   - Added Windows `.exe` binary recognition and stem matching.
   - Typed Unix/Windows path parsing uses configured installation roots and reviewed user subdirectories. It recognizes verbatim paths and ASCII case variants, and rejects prefix lookalikes such as `Program Files-evil` and `.cargo/bin-evil`.
   - Discovery and termination share one typed SID comparison. Missing or different SIDs fail closed; Windows ownership checks do not parse SIDs as numbers.
   - Retained strict fail-closed CWD matching and directory traversal rejection.
   - Fixture tests cover Korean user names and spaces (`D:\Users\홍 길동\.cargo\bin\codex.exe`), lookalike prefixes, and traversal rejection. These are simulated paths; no run on a real Korean-locale machine is recorded yet.

3. **App Uninstaller & Inventory Boundary (#159):**
   - Deleting `Program Files` or trashing Windows directories is prohibited.
   - `PlatformCapabilities::windows()` explicitly marks `installed_apps` and `app_uninstall` as `Unavailable` with an explanatory reason. Zenith does not fabricate or synthesize unverified applications from folder names or assumed `.exe` locations.
   - Backend commands `get_installed_apps`, `inspect_app_uninstall`, and `prepare_app_uninstall` strictly enforce capability requirements and fail closed on Windows with `PlatformCapabilityError::Unavailable`.
   - `StorageView.svelte` omits the Applications tab when `installed_apps` is unavailable, and `ApplicationsView.svelte` renders an informational banner explaining that application inventory is not supported on Windows.

4. **Intensive Cleanup Boundary (#159):**
   - `PlatformCapabilities::windows()` explicitly marks `intensive_cleanup` as `Unavailable` with an explanatory reason (`"Intensive cleanup is unavailable on Windows because no Windows-specific intensive signatures are defined."`).
   - All intensive signatures (`system.intensive.*`) declare `platforms = ["macos"]` and are excluded from the Windows catalog.
   - The intensive cleanup switch in Settings is disabled with an "Unavailable" badge on Windows, and backend cleanup scanning rejects intensive requests on Windows.

Microsoft references: [Known Folders](https://learn.microsoft.com/en-us/windows/win32/shell/known-folders),
[user profiles](https://learn.microsoft.com/en-us/windows/win32/shell/about-user-profiles),
[power request lifecycle](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-powersetrequest).
