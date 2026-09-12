# Windows development

Zenith's Windows build targets x86_64 MSVC and packages as an NSIS installer.
The Windows port is incremental: the shell and capability contract land first,
then each native adapter enables its feature explicitly. Unsupported features
must remain disabled rather than reporting a successful no-op.

## Prerequisites

- Windows 10 version 1809 or newer (Windows 11 recommended)
- Visual Studio 2022 Build Tools with **Desktop development with C++** and the
  Windows 10/11 SDK
- Rust stable MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`).
  The crate declares `rust-version = "1.93.0"`; that is the oldest toolchain the
  locked dependency set builds with.
- Node.js 20 or newer and pnpm 9
- WebView2 Runtime (normally preinstalled on supported Windows versions; the
  installer carries Microsoft's offline installer, so it is not a prerequisite
  for installing a release build)
- NSIS for local installer builds (the Tauri CLI can use its bundled tooling in
  CI)

## Run and verify locally

From PowerShell at the repository root:

```powershell
pnpm install --frozen-lockfile
just lint
just test
just check
just supply-chain
pnpm tauri dev
```

`just` runs on Windows natively. macOS-only recipes such as `just build-fast`,
`just run-fast`, `just distribute`, `just release`, and `just run-bin` are
scoped with a macOS attribute, so they are absent from `just --list` here
instead of failing halfway.

Build unsigned x64 NSIS installers for inspection:

```powershell
pnpm tauri build --debug --bundles nsis
pnpm tauri build --debug --bundles nsis --config .github/tauri.nsis-permachine.json
```

The setup executables are written below `target\debug\bundle\nsis\`.

### Doctor self-check

The binary accepts a first-argument `--doctor` flag that prints a de-identified
environment fingerprint and a self-check table, then exits 0 when every
self-check passes and 1 when one fails. `--doctor --json` prints the same
information as one JSON object. It performs no network access, and the
fingerprint is built from shapes, classifications, and booleans — never a user
name, machine name, drive letter, or profile path.

```powershell
just doctor                          # builds and runs the source tree
& "$env:LOCALAPPDATA\Zenith\Zenith.exe" --doctor
& "C:\Program Files\Zenith\Zenith.exe" --doctor --json
```

CI runs the packaging gate through `just test-package <installer>`, which
installs silently, runs `--doctor`, requires exit code 0 and zero failing
checks, uninstalls silently, and fails if the install directory survives.

## Windows release contract

`src-tauri/tauri.conf.json` keeps the per-user installation as the default
package contract, and the release workflow additionally builds a machine-wide
variant from `.github/tauri.nsis-permachine.json`.

| Artifact | Install mode | Location | Elevation |
| --- | --- | --- | --- |
| `Zenith-windows-x64-setup.exe` | `currentUser` (default) | `%LOCALAPPDATA%\Zenith` | none |
| `Zenith-windows-x64-setup-machine.exe` | `perMachine` | `Program Files\Zenith` | required |

Why both exist:

- The per-user installer is the default because it requires no elevation and
  keeps installation metadata under `HKCU`.
- The machine-wide installer exists for managed environments whose
  application-control policy refuses to execute binaries from user-writable
  locations. AppLocker's default rules and Smart App Control deny execution
  under `%LOCALAPPDATA%`, so a per-user-only package cannot run there at all.
  Machine-wide installation into `Program Files` clears that specific policy
  boundary, **but it does not make the binary trusted**: Smart App Control and
  WDAC still refuse an unsigned or unreputable binary regardless of location,
  and a managed machine may block it entirely.

Further contract points:

- Both installers embed Microsoft's WebView2 **offline** installer
  (`webviewInstallMode: offlineInstaller`, `silent: false`). Installation needs
  no network access and the download grows by roughly 130 MB. With
  `silent: false`, a WebView2 installer failure is shown during installation
  with the vendor's own interface instead of being suppressed and deferred to a
  first launch that never opens a window.
- Downgrades are blocked; installing a newer version upgrades the same
  installation scope.
- The public asset names are always `Zenith-windows-x64-setup.exe` and
  `Zenith-windows-x64-setup-machine.exe`. The WinGet manifest references only
  the per-user installer.
- WebView2 is expected to be present on supported Windows 10 and Windows 11
  installations, but it is not assumed: the embedded offline installer covers a
  machine that has it missing.

## Windows release trust

The Windows beta is deliberately unsigned. Its GitHub Release,
`BUILD_INFO-windows-x64.txt`, and download documentation disclose the
unknown-publisher state. Users verify `SHA256SUMS.txt` before overriding
Microsoft Defender SmartScreen. A self-signed certificate is not an acceptable
public-release substitute.

Because SignPath Foundation issues organization-validated certificates that
carry no SmartScreen reputation, the unknown-publisher warning is expected to
persist after the first signed release. No document or release note may imply
that signing alone removes it.

Release trust artifacts:

- `SHA256SUMS.txt` combined from the platform manifests and the SBOM manifest,
  verified with `shasum -c` against the downloaded bytes before publication. The
  Windows manifest covers both installers.
- `SBOM-zenith.spdx.json`, an SPDX software bill of materials generated from the
  locked dependency manifests of the tagged commit.
- GitHub build provenance attestation for both installers and the SBOM:
  `gh attestation verify Zenith-windows-x64-setup.exe --repo jaeyoung0509/zenith`.
- `endpoint-review.json`, the recorded Microsoft endpoint-protection submission
  result for the exact published bytes. The publishing job runs in the
  `release-approval` environment, where a maintainer submits the installers to
  Microsoft's portal, records `ENDPOINT_REVIEW_STATUS` and the submission
  details, and approves the gate. A recorded detection, a missing or unset
  status, or an artifact whose hash differs from the reviewed bytes blocks
  publication.

Checksum manifests are generated and combined by
`scripts/release_checksums.cjs`. The script writes LF-only text on every runner,
normalizes imported CRLF manifests, and validates their entry shape. The final
publisher must run `shasum -c SHA256SUMS.txt` against every release binary
before uploading any public asset.

After the first installer exists, Zenith can satisfy SignPath Foundation's
"already released" eligibility condition and apply for free open-source code
signing. The approval-dependent identifiers must not be guessed or committed.
Once approved, the release workflow will submit the application and both
installers from the tagged GitHub build, verify the timestamped Authenticode
signature, and fail closed before publication if verification does not succeed.
See [`CODE_SIGNING_POLICY.md`](../CODE_SIGNING_POLICY.md).

## No updater and no background network activity

Zenith has no updater and performs no background network activity, so it never
learns on its own that a corrected version exists. The application exposes the
release URL as `PlatformContext.releases_url` and links to it from the
interface; users compare that page's newest tag with the version shown in
Zenith. Fixes ship as new immutable versioned releases, never as replaced
assets.

## GitHub Release and WinGet

A `v*` tag starts one release workflow with a shared verified frontend, separate
macOS ARM64 and Windows x64 native jobs, a provenance attestation job, and one
final publish job behind the `release-approval` environment. The verification
job also emits an SPDX SBOM for the locked dependency set, and the Windows job
emits both installers, platform checksums, build metadata, and a WinGet
multi-file manifest rooted at:

```text
manifests/z/jaeyoung0509/Zenith/<version>/
```

The manifest identifies the per-user NSIS installer as `nullsoft`, uses
`Scope: user`, declares `/S` for silent installation, and references the
immutable versioned GitHub asset URL. Generate the same files locally with:

```powershell
node scripts/generate_winget_manifest.cjs `
  --version 0.3.19 `
  --installer .\Zenith-windows-x64-setup.exe `
  --output .\winget-output
```

Do not submit this transition manifest to `microsoft/winget-pkgs` while its
installer is unsigned. After SignPath approval and a signed release, run
`winget validate --manifest <version-directory>` and Microsoft's
`SandboxTest.ps1` against the public immutable URL. Verify interactive install,
`winget install`, silent install, upgrade, launch, uninstall, Start Menu entry,
and Apps & Features metadata on clean Windows 10 and Windows 11 systems before
opening the community-repository PR.

## Antivirus and application-control settings

Do not add an antivirus exclusion, and do not document one. The only Windows
security setting a Zenith feature may ask the user to change is **Controlled
Folder Access**: the Large Files inspector and Trash plans operate inside
folders that Controlled Folder Access protects, and the application explains
that single setting when an operation is denied. Excluding the install
directory, the executable, or the user profile from antivirus scanning is not
an acceptable workaround.

Application-control policy is a separate boundary. AppLocker, WDAC, and Smart
App Control are managed by the organization that owns the machine; Zenith can
offer the machine-wide installer, but it cannot and does not claim that a
managed machine will allow the binary to run.

## CI contract

`.github/workflows/ci.yml` runs the same checks on `windows-latest`, plus the
packaging gate described under [Doctor self-check](#doctor-self-check), and
uploads both debug NSIS installers as `zenith-windows-x64-nsis-debug`. The
`msrv` job builds with the declared Rust 1.93.0 toolchain and the `supply-chain`
job audits both lockfiles.

What CI does not verify on Windows: redirected user content folders, machines
whose system drive is not `C:`, UNC or domain-joined profiles, non-NTFS
volumes, non-UTF-8 code pages, standard-user installation without elevation,
application-control policy configurations, or a machine with the WebView2
runtime absent under interactive use. The interactive matrix that covers those
situations is a plan in [`WINDOWS_VALIDATION.md`](WINDOWS_VALIDATION.md); until a
run records its application version, Windows build, and date, it is not
evidence.

Release packaging also runs the native Rust gate and consumes only the frontend
artifact produced by its shared verification job.
