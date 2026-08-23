#!/usr/bin/env bash

# Build a release bundle and recover once when Cargo's unpacked registry source
# is incomplete but the cached crate archive can be fetched again. This keeps
# the common path cache-friendly while making `just release` self-healing for
# the specific local-cache corruption Cargo cannot repair on its own.
set -uo pipefail

build_log="$(mktemp "${TMPDIR:-/tmp}/zenith-tauri-build.XXXXXX")"
trap 'rm -f "$build_log"' EXIT

run_build() {
  pnpm tauri build "$@" 2>&1 | tee "$build_log"
  return "${PIPESTATUS[0]}"
}

run_build "$@"
build_status=$?

if (( build_status == 0 )); then
  exit 0
fi
missing_manifest="$(awk -F'`' '/failed to read `.*\/registry\/src\/.*\/Cargo\.toml`/ { manifest = $2 } END { print manifest }' "$build_log")"

if [[ -z "$missing_manifest" ]]; then
  exit "$build_status"
fi

package_dir="$(dirname "$missing_manifest")"
registry_index_dir="$(dirname "$package_dir")"
registry_src_dir="$(dirname "$registry_index_dir")"

if [[ "$missing_manifest" != /* || "$(basename "$registry_src_dir")" != "src" || "$(basename "$(dirname "$registry_src_dir")")" != "registry" || ! -d "$package_dir" || -L "$package_dir" || -L "$registry_index_dir" || -L "$registry_src_dir" ]]; then
  echo "Cargo reported a registry source path that Zenith cannot safely repair: $missing_manifest" >&2
  exit "$build_status"
fi

repair_dir="$(mktemp -d "${TMPDIR:-/tmp}/zenith-cargo-repair.XXXXXX")"
trap 'rm -f "$build_log"; rm -rf "$repair_dir"' EXIT

echo "Detected an incomplete Cargo source cache at $package_dir. Refreshing only that crate and retrying once..." >&2
mv "$package_dir" "$repair_dir/"

if ! cargo fetch --locked --manifest-path src-tauri/Cargo.toml; then
  echo "Cargo could not refresh the incomplete registry source; keeping the original build failure." >&2
  exit "$build_status"
fi

run_build "$@"
