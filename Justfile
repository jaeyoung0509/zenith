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

# Build quick debug binary (최적화 생략으로 가장 빠르게 빌드)
build-fast:
    pnpm build
    cargo build
    @echo ""
    @echo "⚡ Fast debug binary built at: target/debug/Zenith"

# Run fast debug binary directly
run-fast:
    @if [ -f "target/debug/Zenith" ]; then \
        ./target/debug/Zenith; \
    elif [ -f "src-tauri/target/debug/Zenith" ]; then \
        ./src-tauri/target/debug/Zenith; \
    else \
        cargo run; \
    fi

# ------------------------------------------------------------------------------
# 📦 Production Build (배포용 단일 바이너리 / 앱 번들)
# ------------------------------------------------------------------------------

# Build production macOS App bundle & DMG installer (.app / .dmg)
build:
    pnpm tauri build

# Build standalone single release executable binary (최대 최적화)
build-bin:
    pnpm build
    cargo build --release
    @echo ""
    @echo "✅ Standalone release binary built at: target/release/Zenith"
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

# Run the release standalone binary executable directly
run-bin:
    @if [ -f "target/release/Zenith" ]; then \
        ./target/release/Zenith; \
    elif [ -f "src-tauri/target/release/Zenith" ]; then \
        ./src-tauri/target/release/Zenith; \
    else \
        cargo run --release; \
    fi

# ------------------------------------------------------------------------------
# 🧹 Clean & Maintenance
# ------------------------------------------------------------------------------

# Install all project dependencies
install:
    pnpm install

# Clean all build artifacts and node_modules
clean:
    cargo clean
    rm -rf dist node_modules
    @echo "✨ Cleaned build artifacts and cache."
