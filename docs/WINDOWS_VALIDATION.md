# Windows folder and feature validation

The Windows build needs native runtime checks in addition to successful compilation.
Issue #122 addresses profile, drive, known-folder, Unicode picker, and directory
identity defects found in v0.3.0. The old v0.2.0 screenshots describe the separate
verbatim-path and console regressions fixed in #118.

## Scope and account behavior

- Automatic cache and developer scans use the current process account's
  `USERPROFILE`. A competing Git/MSYS `HOME` is not another scan root. Missing or
  invalid profile paths fail closed instead of switching accounts.
- The system storage card uses `SystemDrive`. Mounted volumes still list all
  disks; listing a drive does not authorize scanning or cleaning the entire drive.
- Large Files uses Windows Known Folders for Downloads, Desktop, Documents, and
  Videos (the existing IPC token is `movies`). This honors configured folder
  redirection, including a normal folder on another drive. Only these fixed
  tokens are accepted; the frontend does not submit arbitrary paths.
- Developer workspace selection remains restricted to children of the current
  profile. Selecting an arbitrary `D:\projects` outside that profile is currently
  unsupported. This is a deliberate existing safety boundary, not drive discovery.
- Junctions, symbolic links, and other reparse points are skipped. Cloud-only
  OneDrive placeholders may therefore be absent; this change does not hydrate or
  traverse them. Use locally available ordinary files for the baseline test.
- Running as another user/admin selects that process account's profile. Zenith
  does not enumerate or clean all accounts on a shared PC.

## Native smoke matrix

Use disposable Windows 10/11 accounts and fixture data. Never use valuable files
for cleanup tests. Record installer version, Windows version, account type,
system drive, and whether folders are redirected; omit secrets from logs.

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

Windows CI covers native Known Folder resolution, real directory identities,
mixed ordinary/verbatim paths, junction rejection, and PowerShell UTF-8 output.
Cross-platform fixture tests cover conflicting account variables, Korean paths,
redirected token scopes, and system-drive selection. These automated checks do
not replace interactive installer/UI and account-switch testing.

## Completed domain adapters (#123)

Issue #123 completes the platform adapter boundaries identified during the initial Windows port:

1. **Keep Awake (`PowerAssertion`):**
   - Migrated from thread-local `SetThreadExecutionState` to owned `PowerCreateRequest`, `PowerSetRequest`, and `PowerClearRequest` handles.
   - Assertions are `Send + Sync`, surviving cross-thread acquisition and release across Tauri blocking-pool and worker threads.
   - Verified via unit tests covering acquisition, display/system request behavior changes, expiration, and multi-thread lifecycle.

2. **AI Activity (`agent_activity::adapters` & `agent_activity::mod`):**
   - Added Windows `.exe` binary recognition and stem matching.
   - Expanded supported install roots to include Windows system locations (`Program Files`, `ProgramData/scoop/shims`, etc.) and user roots (`.cargo\bin`, `AppData\Roaming\npm`, `AppData\Local\Programs`, `scoop\shims`, etc.).
   - Audited process ownership via Windows process token SID matching against Zenith's token SID, ensuring multi-user isolation without failing on non-numeric SIDs.
   - Retained strict fail-closed CWD matching and directory traversal rejection.
   - Verified with Korean usernames and spaces (`D:\Users\홍 길동\.cargo\bin\codex.exe`) as well as lookalike and traversal rejection.

3. **App Uninstaller Boundary:**
   - Deleting `Program Files` or trashing Windows directories is prohibited.
   - `PlatformCapabilities::windows()` explicitly marks `app_uninstall` as `Unavailable` with an explanatory reason, while keeping `installed_apps` available for inventory inspection.
   - `ApplicationsView.svelte` renders an informational banner explaining that uninstallation is not supported on Windows rather than prompting for a trash review.
   - Backend `prepare_app_uninstall` and `trash_manager::from_app_inspection` fail closed on Windows.

Microsoft references: [Known Folders](https://learn.microsoft.com/en-us/windows/win32/shell/known-folders),
[user profiles](https://learn.microsoft.com/en-us/windows/win32/shell/about-user-profiles),
[power request lifecycle](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-powersetrequest).
