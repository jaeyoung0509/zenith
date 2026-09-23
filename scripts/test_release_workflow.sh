#!/usr/bin/env bash

set -euo pipefail

readonly repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/zenith-release-workflow-test.XXXXXX")"
trap 'rm -rf -- "$fixture_root"' EXIT HUP INT TERM

fail() {
  echo "❌ $*" >&2
  exit 1
}

# Check the actual Just recipe without stopping an app or touching /Applications.
recipe="$(cd "$repo_root" && just --dry-run release 2>&1)"
[[ "$recipe" == *"./scripts/tauri_release_build.sh --bundles app"* ]] || fail "release did not request an app-only build"
[[ "$recipe" == *"./scripts/install_release_app.sh"* ]] || fail "release did not invoke the installer"
[[ "$(printf '%s\n' "$recipe" | grep -c '^./scripts/tauri_release_build.sh')" == "1" ]] || fail "release invoked the build more than once"
if printf '%s\n' "$recipe" | grep -qx './scripts/tauri_release_build.sh'; then
  fail "release invoked distribution packaging"
fi
[[ "$recipe" == *"--bundles app"*"./scripts/install_release_app.sh"* ]] || fail "release installed before the build"

mkdir -p "$fixture_root/bin" "$fixture_root/logs"
cat > "$fixture_root/bin/pnpm" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$ZENITH_FAKE_PNPM_CALLS"
if [[ "$ZENITH_FAKE_PNPM_STATUS" == "0" ]]; then
  echo "fixture build succeeded"
else
  echo "fixture build failed" >&2
fi
exit "$ZENITH_FAKE_PNPM_STATUS"
EOF
chmod +x "$fixture_root/bin/pnpm"

export ZENITH_FAKE_PNPM_CALLS="$fixture_root/calls"
export PATH="$fixture_root/bin:$PATH"
export TMPDIR="$fixture_root/logs"
export ZENITH_FAKE_PNPM_STATUS=17
build_status=0
output="$(cd "$repo_root" && ./scripts/tauri_release_build.sh --bundles app 2>&1)" || build_status=$?
[[ "$build_status" == "17" ]] || fail "a failed app build returned $build_status instead of 17"
[[ "$output" == *"Tauri build failed (exit 17). Local build log:"* ]] || fail "failure did not name the stage and retained log"
failure_log="${output##*Local build log: }"
[[ -f "$failure_log" ]] || fail "failed build log was removed"
grep -q "fixture build failed" "$failure_log" || fail "failed build output was not retained"
[[ "$(cat "$ZENITH_FAKE_PNPM_CALLS")" == "tauri build --bundles app" ]] || fail "unexpected fake build command"

export ZENITH_FAKE_PNPM_STATUS=0
success_output="$(cd "$repo_root" && ./scripts/tauri_release_build.sh --bundles app 2>&1)"
[[ "$success_output" == *"fixture build succeeded"* ]] || fail "successful build output was lost"
[[ "$(find "$TMPDIR" -type f -name 'zenith-tauri-build.*' | wc -l | tr -d ' ')" == "1" ]] || fail "successful build retained a log"

echo "✅ Release recipe and build-log regression tests passed."
