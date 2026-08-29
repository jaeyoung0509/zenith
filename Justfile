# Zenith - macOS AI & Developer System Manager Justfile
# https://github.com/casey/just

# Default recipe: Show available commands
default:
    @just --list

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
build-fast:
    pnpm tauri build --debug --bundles app
    @echo ""
    @echo "⚡ Debug app built at: target/debug/bundle/macos/Zenith.app"

# Run fast debug binary directly
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
distribute: stop clean-bin
    ./scripts/tauri_release_build.sh
    @echo ""
    @echo "📦 Fresh release packages built successfully:"
    @echo "  - App Bundle: target/release/bundle/macos/Zenith.app"
    @echo "  - DMG Installer: target/release/bundle/dmg/"
    @echo "👉 Run directly with: just run-bin"

# Build, validate, and safely replace the installed /Applications/Zenith.app.
release: distribute install-release

# Install an already-built release bundle with rollback on replacement failure.
install-release:
    ./scripts/install_release_app.sh

# Build, replace the installed app, and launch that installed copy.
release-and-run: release
    @echo "🚀 Launching installed release..."
    @open "/Applications/Zenith.app"

# Clean existing binaries and build fresh standalone release macOS App bundle
release-app: stop clean-bin
    ./scripts/tauri_release_build.sh --bundles app
    @echo ""
    @echo "✅ Standalone release App built at: target/release/bundle/macos/Zenith.app"
    @echo "👉 Run directly with: just run-bin"

# Build fresh standalone release app and launch immediately
release-app-and-run: release-app
    @echo "🚀 Launching fresh release build..."
    @just run-bin

# Build production macOS App bundle & DMG installer (.app / .dmg)
build:
    pnpm tauri build

# Build standalone release macOS App bundle with full Dock/Finder branding
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
generate-bindings:
    mkdir -p dist
    cargo test --manifest-path src-tauri/Cargo.toml --lib tests::export_typescript_bindings -- --ignored --exact
    @echo "✨ Generated TypeScript bindings at: src/lib/bindings/tauri.ts"

# Run all test suites (Backend Rust Safety + Frontend Vitest)
test: test-rust test-front test-release-installer
    @echo "🎉 All Rust & Frontend tests passed!"

# Run Rust safety invariants & unit tests
test-rust:
    cargo test

# Run frontend Vitest unit tests
test-front:
    pnpm test

# Exercise release replacement and rollback using temporary fixture bundles only.
test-release-installer:
    ./scripts/test_install_release_app.sh

# Check code types & compile check
check:
    cargo check
    pnpm check
    pnpm build

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
stop:
    @-killall Zenith 2>/dev/null || true

# Install all project dependencies
install:
    pnpm install

# Clean previous built binary, app bundles, dmg packages, and dist frontend
clean-bin:
    rm -rf dist target/release/bundle target/release/Zenith target/debug/bundle target/debug/Zenith src-tauri/target/release/bundle src-tauri/target/release/Zenith src-tauri/target/debug/bundle src-tauri/target/debug/Zenith
    @echo "🗑️ Existing binary and bundle artifacts removed."

# Clean all build artifacts, Cargo target, and node_modules
clean:
    cargo clean
    rm -rf dist node_modules
    @echo "✨ Cleaned build artifacts and cache."
