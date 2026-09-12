# Zenith task runner
# https://github.com/casey/just
#
# Cross-platform recipes run on macOS, Linux, and Windows. Recipes that only
# work on one platform are scoped with a platform attribute ([macos] /
# [windows]) so `just --list` shows only what the current host can run.
# CI invokes these recipes instead of repeating command lines.

set shell := ["bash", "-uc"]
set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# Default recipe: Show available commands
default:
    @just --list

# Create the Tauri frontend output directory required by cargo/tauri builds
ensure-dist:
    @node -e "require('fs').mkdirSync('dist', { recursive: true })"

# ------------------------------------------------------------------------------
# 🚀 Development & Fast Run
# ------------------------------------------------------------------------------

# Run full desktop application in development mode (hot-reload)
dev:
    pnpm tauri dev

# Run frontend only in browser (fast UI prototyping with mock IPC)
dev-web:
    pnpm dev

# Build a debug macOS app bundle so Finder and Dock use Zenith branding.
[macos]
build-fast:
    pnpm tauri build --debug --bundles app
    @echo ""
    @echo "⚡ Debug app built at: target/debug/bundle/macos/Zenith.app"

# Run fast debug binary directly (macOS app bundle)
[macos]
run-fast:
    @if [ -d "target/debug/bundle/macos/Zenith.app" ]; then \
        open "target/debug/bundle/macos/Zenith.app"; \
    elif [ -d "src-tauri/target/debug/bundle/macos/Zenith.app" ]; then \
        open "src-tauri/target/debug/bundle/macos/Zenith.app"; \
    elif [ -f "target/debug/Zenith" ]; then \
        ./target/debug/Zenith; \
    else \
        cargo run; \
    fi

# ------------------------------------------------------------------------------
# 📦 Production Build & Distribution
# ------------------------------------------------------------------------------

# Package-only build: clean existing artifacts and create fresh .app and .dmg outputs.
[macos]
distribute: stop clean-bin
    ./scripts/tauri_release_build.sh
    @echo ""
    @echo "📦 Fresh release packages built successfully:"
    @echo "  - App Bundle: target/release/bundle/macos/Zenith.app"
    @echo "  - DMG Installer: target/release/bundle/dmg/"
    @echo "👉 Run directly with: just run-bin"

# Build, validate, and safely replace the installed /Applications/Zenith.app.
[macos]
release: distribute install-release

# Install an already-built release bundle with rollback on replacement failure.
[macos]
install-release:
    ./scripts/install_release_app.sh

# Build, replace the installed app, and launch that installed copy.
[macos]
release-and-run: release
    @echo "🚀 Launching installed release..."
    @open "/Applications/Zenith.app"

# Clean existing binaries and build fresh standalone release macOS App bundle
[macos]
release-app: stop clean-bin
    ./scripts/tauri_release_build.sh --bundles app
    @echo ""
    @echo "✅ Standalone release App built at: target/release/bundle/macos/Zenith.app"
    @echo "👉 Run directly with: just run-bin"

# Build fresh standalone release app and launch immediately
[macos]
release-app-and-run: release-app
    @echo "🚀 Launching fresh release build..."
    @just run-bin

# Build production App bundle & DMG installer (.app / .dmg)
build:
    pnpm tauri build

# Build standalone release macOS App bundle with full Dock/Finder branding
[macos]
build-bin:
    pnpm tauri build --bundles app
    @echo ""
    @echo "✅ Standalone release App built at: target/release/bundle/macos/Zenith.app"
    @echo "👉 Run directly with: just run-bin"

# Build frontend static assets into dist/
build-front:
    pnpm build

# ------------------------------------------------------------------------------
# 🧪 Testing & Verification
# ------------------------------------------------------------------------------

# Generate TypeScript bindings from Rust via Tauri Specta
generate-bindings: ensure-dist
    cargo test --manifest-path src-tauri/Cargo.toml --lib tests::export_typescript_bindings -- --ignored --exact
    cargo test --manifest-path src-tauri/Cargo.toml --lib tests::export_platform_capability_golden -- --ignored --exact
    cargo test --manifest-path src-tauri/Cargo.toml --lib tests::export_platform_context_golden -- --ignored --exact
    @echo "✨ Generated TypeScript bindings and golden data at: src/lib/bindings"

# Run all test suites (backend Rust, frontend Vitest, release-installer regression)
test: test-rust test-front test-release-installer
    @echo "🎉 All Rust & Frontend tests passed!"

# Run Rust safety invariants & unit tests (the same command CI runs)
test-rust:
    cargo test --manifest-path src-tauri/Cargo.toml

# Run frontend Vitest unit tests
test-front:
    pnpm test

# The platform-specific recipes that follow keep `just test` working on every host.
#
# Exercise release replacement and rollback using temporary fixture bundles only.
[macos]
test-release-installer:
    ./scripts/test_install_release_app.sh

[linux]
test-release-installer:
    @echo "The release-replacement regression test uses macOS bundle semantics; nothing to run on Linux."

[windows]
test-release-installer:
    @echo "The release-replacement regression test uses macOS bundle semantics; Windows packaging is covered by 'just test-package <installer>'."

# Install NSIS silently, run --doctor, assert self-checks pass, then uninstall.
# The scope is asserted so a package whose install mode drifted fails here.
[windows]
test-package installer scope="perUser":
    powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File scripts/windows_packaging_smoke.ps1 -InstallerPath "{{installer}}" -Scope "{{scope}}"

# Run the doctor self-check against a binary built from this source tree.
doctor: ensure-dist
    cargo run --manifest-path src-tauri/Cargo.toml --bin Zenith -- --doctor

# Rust format and lint gate (the same command the CI Rust jobs run).
lint-rust:
    cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
    cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

# Lint and format gate (Rust plus the frontend typecheck).
lint: lint-rust
    pnpm check

# Rust compile check (the same command the CI Rust jobs run).
check-rust:
    cargo check --manifest-path src-tauri/Cargo.toml

# Check code types & compile check
check: ensure-dist lint-rust
    just check-rust
    pnpm check
    pnpm build

# Build with the exact rust-version declared in src-tauri/Cargo.toml
check-msrv: ensure-dist
    node scripts/check_rust_version.cjs
    cargo check --manifest-path src-tauri/Cargo.toml --locked

# Fail on locked-dependency vulnerabilities, license drift, and duplicate bans
supply-chain:
    cargo deny check advisories bans licenses sources
    cargo audit
    pnpm audit --audit-level=high

# Check version consistency across package.json, Cargo.toml, and tauri.conf.json
check-version:
    @node scripts/bump_version.cjs check

# Display current application version
version: check-version

# Bump patch version (e.g. 0.1.5 -> 0.1.6) across all manifests
bump-patch:
    @node scripts/bump_version.cjs patch

# Bump minor version (e.g. 0.1.5 -> 0.2.0) across all manifests
bump-minor:
    @node scripts/bump_version.cjs minor

# Bump major version (e.g. 0.1.5 -> 1.0.0) across all manifests
bump-major:
    @node scripts/bump_version.cjs major

# Set an explicit version across all manifests (e.g. just set-version 0.1.5)
set-version version_str:
    @node scripts/bump_version.cjs set {{version_str}}

# ------------------------------------------------------------------------------
# ⚡ Execution
# ------------------------------------------------------------------------------

# Run the release app bundle directly (with full macOS Dock icon)
[macos]
run-bin:
    @if [ -d "target/release/bundle/macos/Zenith.app" ]; then \
        open "target/release/bundle/macos/Zenith.app"; \
    elif [ -d "src-tauri/target/release/bundle/macos/Zenith.app" ]; then \
        open "src-tauri/target/release/bundle/macos/Zenith.app"; \
    elif [ -f "target/release/Zenith" ]; then \
        ./target/release/Zenith; \
    elif [ -f "src-tauri/target/release/Zenith" ]; then \
        ./src-tauri/target/release/Zenith; \
    else \
        pnpm tauri build --bundles app && open "target/release/bundle/macos/Zenith.app"; \
    fi

# ------------------------------------------------------------------------------
# 🧹 Clean & Maintenance
# ------------------------------------------------------------------------------

# Stop running Zenith desktop application instances
[macos]
stop:
    @-killall Zenith 2>/dev/null || true

[windows]
stop:
    -taskkill /IM Zenith.exe /F

[linux]
stop:
    -pkill -x Zenith

# Install all project dependencies
install:
    pnpm install

# Clean previous built binary, app bundles, dmg packages, and dist frontend
clean-bin:
    @node -e "for (const p of ['dist','target/release/bundle','target/release/Zenith','target/debug/bundle','target/debug/Zenith','src-tauri/target/release/bundle','src-tauri/target/release/Zenith','src-tauri/target/debug/bundle','src-tauri/target/debug/Zenith']) require('fs').rmSync(p, { recursive: true, force: true })"
    @echo "🗑️ Existing binary and bundle artifacts removed."

# Clean all build artifacts, Cargo target, and node_modules
clean:
    cargo clean
    @node -e "for (const p of ['dist','node_modules']) require('fs').rmSync(p, { recursive: true, force: true })"
    @echo "✨ Cleaned build artifacts and cache."
