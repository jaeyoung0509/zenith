#!/usr/bin/env node

/**
 * Zenith Version Management Script
 *
 * Synchronizes the application version across package.json,
 * src-tauri/tauri.conf.json, and the Cargo workspace: the version lives once in
 * the root manifest's `[workspace.package]` table, where both `zenith-core` and
 * `zenith-desktop` inherit it.
 */

const fs = require('fs');
const path = require('path');

const rootDir = path.resolve(__dirname, '..');
const pkgPath = path.join(rootDir, 'package.json');
const tauriPath = path.join(rootDir, 'src-tauri', 'tauri.conf.json');
const cargoPath = path.join(rootDir, 'Cargo.toml');
const cargoLockPath = path.join(rootDir, 'Cargo.lock');

/**
 * Workspace members that carry the application version.
 *
 * Cargo.lock is checked for every one of them: the lock lists the two crates
 * alphabetically, so a single `name = "zenith-core"` lookup would match the
 * domain crate and silently stop comparing the application.
 */
const VERSIONED_PACKAGES = ['zenith-core', 'zenith-desktop', 'zenith-platform'];

/** Half-open range of the `[workspace.package]` table inside the manifest. */
function workspacePackageTableRange(manifest) {
  const header = '[workspace.package]';
  const start = manifest.indexOf(header);
  if (start === -1) {
    return null;
  }
  const bodyStart = start + header.length;
  const nextTable = manifest.indexOf('\n[', bodyStart);
  return { start, bodyStart, end: nextTable === -1 ? manifest.length : nextTable };
}

/** Reads `version` out of the root manifest's `[workspace.package]` table. */
function readWorkspaceVersion(manifest) {
  const range = workspacePackageTableRange(manifest);
  if (!range) {
    return null;
  }
  const version = manifest
    .slice(range.bodyStart, range.end)
    .match(/^version\s*=\s*"([^"]+)"/m);
  return version ? version[1] : null;
}

function getVersions() {
  const pkgMatch = fs.readFileSync(pkgPath, 'utf8').match(/"version":\s*"([^"]+)"/);
  const pkg = pkgMatch ? pkgMatch[1] : null;

  const tauriMatch = fs.readFileSync(tauriPath, 'utf8').match(/"version":\s*"([^"]+)"/);
  const tauri = tauriMatch ? tauriMatch[1] : null;

  const cargo = readWorkspaceVersion(fs.readFileSync(cargoPath, 'utf8'));

  const cargoLockText = fs.readFileSync(cargoLockPath, 'utf8');
  const cargoLock = VERSIONED_PACKAGES.map((name) => {
    const match = cargoLockText.match(
      new RegExp(`name = "${name}"\\r?\\nversion = "([^"]+)"`),
    );
    return { name, version: match ? match[1] : null };
  });

  return { pkg, tauri, cargo, cargoLock };
}

function checkVersions(expectedVersion) {
  const { pkg, tauri, cargo, cargoLock } = getVersions();
  console.log(`📦 package.json:                   ${pkg}`);
  console.log(`🦀 Cargo.toml [workspace.package]: ${cargo}`);
  for (const entry of cargoLock) {
    console.log(`🔒 Cargo.lock ${entry.name}:${' '.repeat(Math.max(1, 20 - entry.name.length))}${entry.version}`);
  }
  console.log(`⚙️  tauri.conf.json:                ${tauri}`);

  const lockVersionsMatch =
    cargoLock.length === VERSIONED_PACKAGES.length &&
    cargoLock.every((entry) => entry.version === pkg);

  if (!pkg || !tauri || !cargo || !lockVersionsMatch || pkg !== cargo || pkg !== tauri) {
    console.error('❌ Version mismatch detected between manifest files!');
    process.exit(1);
  }

  if (expectedVersion) {
    const cleanExpected = expectedVersion.trim().replace(/^v/, '');
    if (pkg !== cleanExpected) {
      console.error(`❌ Release version ${cleanExpected} does not match manifest version ${pkg}!`);
      process.exit(1);
    }
  }
  console.log('✅ All manifest versions are synchronized.');
}

function writeVersion(nextVersion) {
  const cleanVersion = nextVersion.trim().replace(/^v/, '');
  if (!/^\d+\.\d+\.\d+(-[a-zA-Z0-9.]+)?$/.test(cleanVersion)) {
    console.error(`❌ Invalid semver version format: ${nextVersion}`);
    process.exit(1);
  }

  // 1. package.json
  let pkg = fs.readFileSync(pkgPath, 'utf8');
  pkg = pkg.replace(/"version":\s*"[^"]+"/, `"version": "${cleanVersion}"`);
  fs.writeFileSync(pkgPath, pkg);

  // 2. src-tauri/tauri.conf.json
  let tauri = fs.readFileSync(tauriPath, 'utf8');
  tauri = tauri.replace(/"version":\s*"[^"]+"/, `"version": "${cleanVersion}"`);
  fs.writeFileSync(tauriPath, tauri);

  // 3. Cargo.toml — the workspace version both members inherit
  const cargo = fs.readFileSync(cargoPath, 'utf8');
  const range = workspacePackageTableRange(cargo);
  if (!range) {
    console.error('❌ Cargo.toml no longer declares a [workspace.package] table');
    process.exit(1);
  }
  const table = cargo.slice(range.bodyStart, range.end);
  const rewritten = table.replace(/^version\s*=\s*"[^"]+"/m, `version = "${cleanVersion}"`);
  if (rewritten === table) {
    console.error('❌ Cargo.toml [workspace.package] has no version to update');
    process.exit(1);
  }
  fs.writeFileSync(
    cargoPath,
    cargo.slice(0, range.bodyStart) + rewritten + cargo.slice(range.end),
  );

  // 4. Cargo.lock — every workspace member that inherits the version
  let cargoLock = fs.readFileSync(cargoLockPath, 'utf8');
  for (const name of VERSIONED_PACKAGES) {
    cargoLock = cargoLock.replace(
      new RegExp(`(name = "${name}"\\r?\\nversion = ")[^"]+"`),
      `$1${cleanVersion}"`,
    );
  }
  fs.writeFileSync(cargoLockPath, cargoLock);

  console.log(`🚀 Successfully updated version to ${cleanVersion} across all manifests.`);
  checkVersions();
}

function bump(type) {
  const { pkg } = getVersions();
  if (!pkg) {
    console.error('❌ Failed to read package.json version');
    process.exit(1);
  }
  const parts = pkg.split('.').map(Number);
  if (parts.length < 3 || parts.some(isNaN)) {
    console.error(`❌ Current version ${pkg} is not standard X.Y.Z semver`);
    process.exit(1);
  }

  if (type === 'patch') {
    parts[2] += 1;
  } else if (type === 'minor') {
    parts[1] += 1;
    parts[2] = 0;
  } else if (type === 'major') {
    parts[0] += 1;
    parts[1] = 0;
    parts[2] = 0;
  } else {
    console.error(`❌ Unknown bump type: ${type}`);
    process.exit(1);
  }

  writeVersion(parts.join('.'));
}

const command = process.argv[2] || 'check';
const arg = process.argv[3];

switch (command) {
  case 'check':
    checkVersions(arg);
    break;
  case 'patch':
  case 'minor':
  case 'major':
    bump(command);
    break;
  case 'set':
    if (!arg) {
      console.error('❌ Missing target version argument for set command');
      process.exit(1);
    }
    writeVersion(arg);
    break;
  default:
    console.error(`❌ Unknown command: ${command}`);
    process.exit(1);
}
