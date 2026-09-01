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

The setup executable is written below
`target\debug\bundle\nsis\`. Release builds must be signed before sharing;
the release workflow owns signing and WinGet publication.

## CI contract

`.github/workflows/ci.yml` runs the same checks on `windows-latest` and uploads
the unsigned NSIS smoke artifact as `zenith-windows-x64-nsis-debug`. The job is
the authoritative Windows compile/package check; macOS remains covered by its
existing job and `just build-fast` bundle verification.
