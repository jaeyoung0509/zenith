# Zenith - macOS AI & Developer System Manager Justfile
# https://github.com/casey/just

# Default recipe: Show available commands
default:
    @just --list

# ------------------------------------------------------------------------------
# 🚀 Development & Fast Run (빠른 개발용)
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
# 📦 Production Build & Distribution (배포용 단일 바이너리 / 앱 번들)
# ------------------------------------------------------------------------------

# Clean existing binaries and build fresh release packages (.app & .dmg)
distribute: clean-bin
    pnpm tauri build
    @echo ""
    @echo "📦 Fresh release packages built successfully:"
    @echo "  - App Bundle: target/release/bundle/macos/Zenith.app"
    @echo "  - DMG Installer: target/release/bundle/dmg/"
    @echo "👉 Run directly with: just run-bin"

# Alias for distribute
release: distribute

# Build production macOS App bundle & DMG installer (.app / .dmg)
build:
    pnpm tauri build

# Clean existing binaries and build fresh standalone release macOS App bundle
release-app: clean-bin
    pnpm tauri build --bundles app
    @echo ""
    @echo "✅ Standalone release App built at: target/release/bundle/macos/Zenith.app"
    @echo "👉 Run directly with: just run-bin"

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

# Run all test suites (Backend Rust Safety + Frontend Vitest)
test: test-rust test-front
    @echo "🎉 All Rust & Frontend tests passed!"

# Run Rust safety invariants & unit tests
test-rust:
    cargo test

# Run frontend Vitest unit tests
test-front:
    pnpm test

# Check code types & compile check
check:
    cargo check
    pnpm build

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
