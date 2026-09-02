#!/usr/bin/env node

/**
 * Zenith Version Management Script
 * Synchronizes versions across package.json, src-tauri/Cargo.toml, and src-tauri/tauri.conf.json.
 */

const fs = require('fs');
const path = require('path');

const rootDir = path.resolve(__dirname, '..');
const pkgPath = path.join(rootDir, 'package.json');
const tauriPath = path.join(rootDir, 'src-tauri', 'tauri.conf.json');
const cargoPath = path.join(rootDir, 'src-tauri', 'Cargo.toml');
const cargoLockPath = path.join(rootDir, 'Cargo.lock');

function getVersions() {
  const pkgMatch = fs.readFileSync(pkgPath, 'utf8').match(/"version":\s*"([^"]+)"/);
  const pkg = pkgMatch ? pkgMatch[1] : null;

  const tauriMatch = fs.readFileSync(tauriPath, 'utf8').match(/"version":\s*"([^"]+)"/);
  const tauri = tauriMatch ? tauriMatch[1] : null;

  const cargoMatch = fs.readFileSync(cargoPath, 'utf8').match(/^version\s*=\s*"([^"]+)"/m);
  const cargo = cargoMatch ? cargoMatch[1] : null;

  const cargoLockMatch = fs
    .readFileSync(cargoLockPath, 'utf8')
    .match(/name = "zenith-core"\r?\nversion = "([^"]+)"/);
  const cargoLock = cargoLockMatch ? cargoLockMatch[1] : null;

  return { pkg, tauri, cargo, cargoLock };
}

function checkVersions(expectedVersion) {
  const { pkg, tauri, cargo, cargoLock } = getVersions();
  console.log(`📦 package.json:      ${pkg}`);
  console.log(`🦀 Cargo.toml:        ${cargo}`);
  console.log(`🔒 Cargo.lock:        ${cargoLock}`);
  console.log(`⚙️  tauri.conf.json:   ${tauri}`);

  if (!pkg || !tauri || !cargo || !cargoLock || pkg !== cargo || pkg !== tauri || pkg !== cargoLock) {
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

  // 3. src-tauri/Cargo.toml
  let cargo = fs.readFileSync(cargoPath, 'utf8');
  cargo = cargo.replace(/^version\s*=\s*"[^"]+"/m, `version = "${cleanVersion}"`);
  fs.writeFileSync(cargoPath, cargo);

  // 4. Cargo.lock workspace package entry
  let cargoLock = fs.readFileSync(cargoLockPath, 'utf8');
  cargoLock = cargoLock.replace(
    /(name = "zenith-core"\r?\nversion = ")[^"]+"/,
    `$1${cleanVersion}"`,
  );
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
