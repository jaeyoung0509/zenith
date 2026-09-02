#!/usr/bin/env bash

# Install a verified release bundle without risking the currently installed app.
# Custom paths and failure injection are intentionally restricted to regression tests.
set -euo pipefail

readonly expected_bundle_id="com.zenith.desktop"
readonly plist_buddy="/usr/libexec/PlistBuddy"
readonly script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly repository_root="$(cd "$script_dir/.." && pwd -P)"

source_app="$repository_root/target/release/bundle/macos/Zenith.app"
applications_dir="/Applications"
launch_after_install=0
custom_path_requested=0
test_mode="${ZENITH_INSTALL_TEST_MODE:-0}"
test_failpoint="${ZENITH_INSTALL_TEST_FAILPOINT:-}"

usage() {
  cat <<'USAGE'
Usage: scripts/install_release_app.sh [--launch]

Installs target/release/bundle/macos/Zenith.app as /Applications/Zenith.app.

Test-only options (require ZENITH_INSTALL_TEST_MODE=1):
  --source <path>            Source Zenith.app fixture
  --applications-dir <path> Destination Applications fixture directory
USAGE
}

fail() {
  echo "❌ $*" >&2
  exit 1
}

while (($# > 0)); do
  case "$1" in
    --launch)
      launch_after_install=1
      shift
      ;;
    --source)
      (($# >= 2)) || fail "--source requires a path."
      source_app="$2"
      custom_path_requested=1
      shift 2
      ;;
    --applications-dir)
      (($# >= 2)) || fail "--applications-dir requires a path."
      applications_dir="$2"
      custom_path_requested=1
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      usage >&2
      fail "Unknown option: $1"
      ;;
  esac
done

if ((custom_path_requested)) && [[ "$test_mode" != "1" ]]; then
  fail "Custom install paths are available only with ZENITH_INSTALL_TEST_MODE=1."
fi
if [[ -n "$test_failpoint" && "$test_mode" != "1" ]]; then
  fail "Failure injection is available only in test mode."
fi
if [[ "$(uname -s)" != "Darwin" ]]; then
  fail "The Zenith app installer supports macOS only."
fi
if [[ "$(basename "$source_app")" != "Zenith.app" ]]; then
  fail "The source bundle must be named exactly Zenith.app."
fi
if [[ ! -d "$source_app" || -L "$source_app" ]]; then
  fail "Release bundle is missing or is a symlink: $source_app"
fi
if [[ ! -d "$applications_dir" || -L "$applications_dir" ]]; then
  fail "Applications directory is missing or is a symlink: $applications_dir"
fi
if [[ ! -x "$plist_buddy" ]]; then
  fail "PlistBuddy is required to validate the application bundle."
fi

readonly source_parent="$(cd "$(dirname "$source_app")" && pwd -P)"
source_app="$source_parent/Zenith.app"
applications_dir="$(cd "$applications_dir" && pwd -P)"
readonly destination_app="$applications_dir/Zenith.app"

if [[ "$source_app" == "$destination_app" ]]; then
  fail "Source and installed application paths must be different."
fi

bundle_value() {
  local app_path="$1"
  local key="$2"
  local plist_path="$app_path/Contents/Info.plist"
  [[ -f "$plist_path" && ! -L "$plist_path" ]] || return 1
  "$plist_buddy" -c "Print :$key" "$plist_path" 2>/dev/null
}

validate_bundle() {
  local app_path="$1"
  local identifier
  local version

  [[ -d "$app_path" && ! -L "$app_path" ]] || return 1
  identifier="$(bundle_value "$app_path" CFBundleIdentifier)" || return 1
  version="$(bundle_value "$app_path" CFBundleShortVersionString)" || return 1
  [[ "$identifier" == "$expected_bundle_id" && -n "$version" ]]
}

validate_bundle "$source_app" || fail "The release bundle is invalid or does not identify as $expected_bundle_id."
readonly built_version="$(bundle_value "$source_app" CFBundleShortVersionString)"

if [[ -e "$destination_app" || -L "$destination_app" ]]; then
  validate_bundle "$destination_app" || fail "Refusing to replace an unverified bundle at $destination_app."
fi

stage_dir=""
cleanup_stage() {
  if [[ -n "$stage_dir" && -d "$stage_dir" && "$stage_dir" == "$applications_dir"/.zenith-install.* ]]; then
    rm -rf -- "$stage_dir"
  fi
}
trap cleanup_stage EXIT HUP INT TERM

if ! stage_dir="$(mktemp -d "$applications_dir/.zenith-install.XXXXXX")"; then
  fail "Cannot prepare an install transaction in $applications_dir. Check directory permissions."
fi
readonly staged_app="$stage_dir/new.app"
readonly previous_app="$stage_dir/previous.app"
readonly failed_app="$stage_dir/failed.app"
transaction_active=0
new_activated=0
had_previous=0

finish_transaction() {
  local exit_status=$?
  trap - EXIT HUP INT TERM

  if ((transaction_active)); then
    if ((new_activated)) && [[ -e "$destination_app" || -L "$destination_app" ]]; then
      if ! mv -- "$destination_app" "$failed_app"; then
        echo "❌ Automatic rollback could not quarantine the failed app; the previous app remains at $previous_app." >&2
        stage_dir=""
        exit 1
      fi
    fi
    if ((had_previous)) && [[ -d "$previous_app" ]]; then
      if [[ ! -e "$destination_app" ]]; then
        mv -- "$previous_app" "$destination_app" || {
          echo "❌ Automatic rollback failed; the previous app remains at $previous_app." >&2
          exit_status=1
          stage_dir=""
        }
      fi
    fi
  fi

  cleanup_stage
  exit "$exit_status"
}
trap finish_transaction EXIT
trap 'exit 130' HUP INT TERM

if [[ "$test_failpoint" == "before-copy" ]]; then
  fail "Injected failure before staging the new bundle."
fi
if ! /usr/bin/ditto "$source_app" "$staged_app"; then
  fail "Could not stage the new bundle. The installed app was not changed."
fi
validate_bundle "$staged_app" || fail "The staged bundle failed validation. The installed app was not changed."
readonly staged_version="$(bundle_value "$staged_app" CFBundleShortVersionString)"
[[ "$staged_version" == "$built_version" ]] || fail "The staged bundle version does not match the build."

if [[ "$test_mode" != "1" ]]; then
  killall Zenith 2>/dev/null || true
fi

transaction_active=1
if [[ -e "$destination_app" ]]; then
  if ! mv -- "$destination_app" "$previous_app"; then
    fail "Could not move the installed app into the rollback area. Check /Applications permissions."
  fi
  had_previous=1
fi

if [[ "$test_failpoint" == "after-backup" ]]; then
  fail "Injected failure after backing up the installed bundle; the previous app was restored."
fi

if ! mv -- "$staged_app" "$destination_app"; then
  fail "Could not activate the new bundle. The previous app was restored."
fi
new_activated=1

if [[ "$test_failpoint" == "after-activation" ]]; then
  fail "Injected failure after activating the new bundle; the previous app was restored."
fi

if ! validate_bundle "$destination_app"; then
  fail "Installed-bundle validation failed. The previous app was restored."
fi

readonly installed_version="$(bundle_value "$destination_app" CFBundleShortVersionString)"
if [[ "$installed_version" != "$built_version" ]]; then
  fail "Installed version $installed_version does not match built version $built_version. The previous app was restored."
fi

transaction_active=0
if ((had_previous)); then
  rm -rf -- "$previous_app"
fi

echo "✅ Zenith installed successfully"
echo "  Built version:     $built_version"
echo "  Installed version: $installed_version"
echo "  Destination:       $destination_app"

if ((launch_after_install)); then
  if ! open "$destination_app"; then
    fail "Installation succeeded, but macOS could not launch $destination_app."
  fi
fi
