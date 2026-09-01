# Windows development

Zenith's Windows build targets x86_64 MSVC and packages as an NSIS installer.
The Windows port is incremental: the shell and capability contract land first,
then each native adapter enables its feature explicitly. Unsupported features
must remain disabled rather than reporting a successful no-op.

## Prerequisites

- Windows 10 version 1809 or newer (Windows 11 recommended)
- Visual Studio 2022 Build Tools with **Desktop development with C++** and the
  Windows 10/11 SDK
- Rust stable MSVC toolchain (`rustup default stable-x86_64-pc-windows-msvc`)
- Node.js 20 or newer and pnpm 9
- WebView2 Runtime (normally preinstalled on supported Windows versions)
- NSIS for local installer builds (the Tauri CLI can use its bundled tooling in
  CI)

## Run and verify locally

From PowerShell at the repository root:

```powershell
pnpm install --frozen-lockfile
pnpm check
pnpm test -- --run
pnpm build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
pnpm tauri dev
```

Build an unsigned x64 NSIS installer for inspection:

```powershell
pnpm tauri build --debug --bundles nsis
```

The setup executable is written below `target\debug\bundle\nsis\`.

## Windows release contract

`src-tauri/tauri.conf.json` makes the Windows package contract explicit:

- x64 NSIS installs for the current user and writes installer metadata under
  `HKCU`; installation does not request administrator rights.
- WebView2 uses Tauri's silent download bootstrapper mode. Supported Windows
  10 and Windows 11 installations normally already contain the runtime.
- Downgrades are blocked, while installing a newer version upgrades the same
  current-user application.
- The public asset name is always `Zenith-windows-x64-setup.exe`.

The first v0.2.0 Windows beta is deliberately unsigned. Its GitHub Release,
`BUILD_INFO-windows-x64.txt`, and download documentation all disclose the
unknown-publisher state. Users should verify `SHA256SUMS.txt` before overriding
Microsoft Defender SmartScreen. A self-signed certificate is not an acceptable
public-release substitute.

After that first installer exists, Zenith can satisfy SignPath Foundation's
"already released" eligibility condition and apply for free open-source code
signing. The approval-dependent identifiers must not be guessed or committed.
Once approved, the release workflow will submit the application and installer
from the tagged GitHub build, verify the timestamped Authenticode signature,
and fail closed before publication if verification does not succeed. See
[`CODE_SIGNING_POLICY.md`](../CODE_SIGNING_POLICY.md).

## GitHub Release and WinGet

A `v*` tag starts one release workflow with a shared verified frontend, separate
macOS ARM64 and Windows x64 native jobs, and one final publish job. The Windows
job emits the installer, platform checksum, build metadata, and a WinGet
multi-file manifest rooted at:

```text
manifests/z/jaeyoung0509/Zenith/<version>/
```

The manifest identifies the NSIS installer as `nullsoft`, uses `Scope: user`,
declares `/S` for silent installation, and references the immutable versioned
GitHub asset URL. Generate the same files locally with:

```powershell
node scripts/generate_winget_manifest.cjs `
  --version 0.2.0 `
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

## CI contract

`.github/workflows/ci.yml` runs the same checks on `windows-latest` and uploads
the unsigned NSIS smoke artifact as `zenith-windows-x64-nsis-debug`. The job is
the authoritative Windows compile/package check; macOS remains covered by its
existing job and `just build-fast` bundle verification. Release packaging also
runs the native Rust gate and consumes only the frontend artifact produced by
its shared verification job.
