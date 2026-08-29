#!/usr/bin/env bash

set -euo pipefail

readonly script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly installer="$script_dir/install_release_app.sh"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/zenith-install-test.XXXXXX")"
trap 'rm -rf -- "$fixture_root"' EXIT HUP INT TERM

fail() {
  echo "❌ $*" >&2
  exit 1
}

make_bundle() {
  local app_path="$1"
  local version="$2"
  local identifier="$3"
  local marker="$4"

  mkdir -p "$app_path/Contents/MacOS"
  plutil -create xml1 "$app_path/Contents/Info.plist"
  plutil -insert CFBundleIdentifier -string "$identifier" "$app_path/Contents/Info.plist"
  plutil -insert CFBundleShortVersionString -string "$version" "$app_path/Contents/Info.plist"
  plutil -insert CFBundleExecutable -string Zenith "$app_path/Contents/Info.plist"
  printf '%s\n' "$marker" > "$app_path/Contents/MacOS/Zenith"
  chmod +x "$app_path/Contents/MacOS/Zenith"
}

assert_installed() {
  local applications_dir="$1"
  local expected_version="$2"
  local expected_marker="$3"
  local installed="$applications_dir/Zenith.app"
  local actual_version
  local actual_marker

  actual_version="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$installed/Contents/Info.plist")"
  actual_marker="$(tr -d '\n' < "$installed/Contents/MacOS/Zenith")"
  [[ "$actual_version" == "$expected_version" ]] || fail "Expected version $expected_version, got $actual_version."
  [[ "$actual_marker" == "$expected_marker" ]] || fail "Expected marker $expected_marker, got $actual_marker."
}

run_installer() {
  local source_app="$1"
  local applications_dir="$2"
  shift 2
  ZENITH_INSTALL_TEST_MODE=1 "$@" "$installer" --source "$source_app" --applications-dir "$applications_dir"
}

test_replaces_an_older_bundle() {
  local test_root="$fixture_root/replacement"
  local source_app="$test_root/build/Zenith.app"
  local applications_dir="$test_root/Applications"
  mkdir -p "$applications_dir"
  make_bundle "$source_app" 0.1.18 com.zenith.desktop new-build
  make_bundle "$applications_dir/Zenith.app" 0.1.17 com.zenith.desktop old-install

  local output
  output="$(run_installer "$source_app" "$applications_dir" env)"

  assert_installed "$applications_dir" 0.1.18 new-build
  [[ "$output" == *"Built version:     0.1.18"* ]] || fail "Installer did not report the built version."
  [[ "$output" == *"Installed version: 0.1.18"* ]] || fail "Installer did not report the installed version."
  [[ -z "$(find "$applications_dir" -mindepth 1 -maxdepth 1 -name '.zenith-install.*' -print -quit)" ]] || fail "Installer left a transaction directory behind."
  [[ "$(find "$applications_dir" -mindepth 1 -maxdepth 1 -type d -name '*Zenith*.app' | wc -l | tr -d ' ')" == "1" ]] || fail "Installer created a stale Zenith bundle."
}

test_installs_when_no_previous_bundle_exists() {
  local test_root="$fixture_root/first-install"
  local source_app="$test_root/build/Zenith.app"
  local applications_dir="$test_root/Applications"
  mkdir -p "$applications_dir"
  make_bundle "$source_app" 0.1.18 com.zenith.desktop first-install

  run_installer "$source_app" "$applications_dir" env >/dev/null

  assert_installed "$applications_dir" 0.1.18 first-install
}

test_rejects_wrong_source_identifier() {
  local test_root="$fixture_root/wrong-source"
  local source_app="$test_root/build/Zenith.app"
  local applications_dir="$test_root/Applications"
  mkdir -p "$applications_dir"
  make_bundle "$source_app" 0.1.18 example.untrusted.app untrusted
  make_bundle "$applications_dir/Zenith.app" 0.1.17 com.zenith.desktop old-install

  if run_installer "$source_app" "$applications_dir" env >/dev/null 2>&1; then
    fail "Installer accepted a source with the wrong bundle identifier."
  fi
  assert_installed "$applications_dir" 0.1.17 old-install
}

test_rejects_unverified_destination() {
  local test_root="$fixture_root/wrong-destination"
  local source_app="$test_root/build/Zenith.app"
  local applications_dir="$test_root/Applications"
  mkdir -p "$applications_dir"
  make_bundle "$source_app" 0.1.18 com.zenith.desktop new-build
  make_bundle "$applications_dir/Zenith.app" 9.9.9 example.other.app other-app

  if run_installer "$source_app" "$applications_dir" env >/dev/null 2>&1; then
    fail "Installer replaced an unverified destination bundle."
  fi
  assert_installed "$applications_dir" 9.9.9 other-app
}

test_copy_failure_preserves_previous_bundle() {
  local test_root="$fixture_root/copy-failure"
  local source_app="$test_root/build/Zenith.app"
  local applications_dir="$test_root/Applications"
  mkdir -p "$applications_dir"
  make_bundle "$source_app" 0.1.18 com.zenith.desktop new-build
  make_bundle "$applications_dir/Zenith.app" 0.1.17 com.zenith.desktop old-install

  if run_installer "$source_app" "$applications_dir" env ZENITH_INSTALL_TEST_FAILPOINT=before-copy >/dev/null 2>&1; then
    fail "Injected staging failure unexpectedly succeeded."
  fi
  assert_installed "$applications_dir" 0.1.17 old-install
}

test_permission_failure_preserves_previous_bundle() {
  local test_root="$fixture_root/permission-failure"
  local source_app="$test_root/build/Zenith.app"
  local applications_dir="$test_root/Applications"
  local error_log="$test_root/error.log"
  mkdir -p "$applications_dir"
  make_bundle "$source_app" 0.1.18 com.zenith.desktop new-build
  make_bundle "$applications_dir/Zenith.app" 0.1.17 com.zenith.desktop old-install
  chmod 500 "$applications_dir"

  if run_installer "$source_app" "$applications_dir" env > /dev/null 2> "$error_log"; then
    chmod 700 "$applications_dir"
    fail "Installer unexpectedly succeeded without destination write permission."
  fi
  chmod 700 "$applications_dir"

  grep -q "Check directory permissions" "$error_log" || fail "Permission failure did not provide an actionable error."
  assert_installed "$applications_dir" 0.1.17 old-install
}

test_activation_failure_rolls_back() {
  local test_root="$fixture_root/rollback"
  local source_app="$test_root/build/Zenith.app"
  local applications_dir="$test_root/Applications"
  mkdir -p "$applications_dir"
  make_bundle "$source_app" 0.1.18 com.zenith.desktop new-build
  make_bundle "$applications_dir/Zenith.app" 0.1.17 com.zenith.desktop old-install

  if run_installer "$source_app" "$applications_dir" env ZENITH_INSTALL_TEST_FAILPOINT=after-activation >/dev/null 2>&1; then
    fail "Injected activation failure unexpectedly succeeded."
  fi
  assert_installed "$applications_dir" 0.1.17 old-install
}

test_custom_paths_require_test_mode() {
  local test_root="$fixture_root/path-gate"
  local source_app="$test_root/build/Zenith.app"
  local applications_dir="$test_root/Applications"
  mkdir -p "$applications_dir"
  make_bundle "$source_app" 0.1.18 com.zenith.desktop new-build

  if "$installer" --source "$source_app" --applications-dir "$applications_dir" >/dev/null 2>&1; then
    fail "Installer accepted custom paths outside test mode."
  fi
  [[ ! -e "$applications_dir/Zenith.app" ]] || fail "Path-gate test unexpectedly installed an app."
}

test_replaces_an_older_bundle
test_installs_when_no_previous_bundle_exists
test_rejects_wrong_source_identifier
test_rejects_unverified_destination
test_copy_failure_preserves_previous_bundle
test_permission_failure_preserves_previous_bundle
test_activation_failure_rolls_back
test_custom_paths_require_test_mode

echo "✅ Release installer regression tests passed."
