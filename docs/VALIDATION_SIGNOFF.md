# Cleaner V2 validation sign-off

This page states what each evidence source proves. A green automated job is not
substituted for a supported desktop run, and a read-only observation is not
substituted for a mutation test.

## Evidence classes

| Evidence class | Current source | What it proves | What it does not prove |
|---|---|---|---|
| Simulated and disposable fixtures | `scan_benchmark`, environment fixtures, safety and cleanup tests | Deterministic traversal facts, cancellation, overlap handling, path algebra, safety refusals, and mutation outcomes against disposable test data | Behavior of a user's real profile, tool installation, permissions, or desktop shell |
| Native CI API checks | macOS and Windows Rust jobs plus packaging smoke | Platform code compiles and the documented native API, junction, handle, and installer assertions execute on hosted runners | A supported Windows 11 desktop configuration or interactive user workflow |
| Real-machine read-only observation | [macOS 0.3.48 record](validation/2026-09-22-macos-0.3.48.json) | Apple M1 filesystem traversal, aggregate pnpm/npm cache inspection, a typed uv failure, provider progress/cancellation, a stopped Docker daemon, and OrbStack metadata | Cleanup, provider/container prune, Recycle Bin mutation, zero-item Cursor/Go paths, a running Docker daemon, or Windows behavior |
| Disposable-fixture mutation | Automated cleanup/Trash/provider tests | Authorized mutation is revalidated and reported for controlled fixtures | Permission prompts, live Recycle Bin UX, locked application state, or valuable real-user data |
| Supported desktop validation | [Windows manual matrix](WINDOWS_VALIDATION.md) | Nothing yet: the matrix remains a plan | All Windows 11 acceptance rows until a dated machine record is committed |

## 0.3.48 decision

The 0.3.48 macOS run closes neither a typed failure nor a zero-item case by
renaming it as success. The ordinary filesystem result is `partial`; uv reports
an `io_error`; Docker was installed but its daemon was not running; Cursor and
Go produced no candidates. pnpm and npm produced fresh aggregate observations,
and the provider cancellation run stopped after the first root progress event
with a typed cancelled result.

No cleanup plan or executor was constructed by the evidence tool. Its optional
container mode uses fixed-argument status/list inspection and local OrbStack
metadata; its provider mode uses fixed-argument cache-directory discovery and
measurement. Paths, item names, command output, free-form errors, and
`DOCKER_HOST` values are omitted from the record.

## Remaining sign-off

- #224 remains open until a supported Windows 11 desktop record covers its
  functional, performance, provider, and disposable-fixture checklist or a
  deliberate product-scope decision moves every deferred row to owned work.
- #227 remains open for the linked Windows record and the macOS cases that were
  unavailable or produced no items in 0.3.48.
- #221 remains open while either native-validation issue remains open.

Future records must identify the Zenith version, exact OS build, UTC date,
hardware, privilege/profile facts where relevant, and whether the operation was
simulated, read-only, or a disposable-fixture mutation. Partial and unavailable
results stay partial and unavailable.
