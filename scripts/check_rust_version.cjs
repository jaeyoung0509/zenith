#!/usr/bin/env node

/**
 * Verifies that the active Rust toolchain matches the `rust-version` declared
 * in src-tauri/Cargo.toml.
 *
 * The `msrv` CI job installs exactly that toolchain before running this script,
 * so a toolchain/declaration drift fails the job instead of silently verifying
 * a newer compiler than the project claims to support.
 */

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const rootDir = path.resolve(__dirname, '..');
const manifestPath = path.join(rootDir, 'src-tauri', 'Cargo.toml');

function fail(message) {
  console.error(`Error: ${message}`);
  process.exit(1);
}

const manifest = fs.readFileSync(manifestPath, 'utf8');
const declared = manifest.match(/^rust-version\s*=\s*"([^"]+)"/m)?.[1];

if (!declared) {
  fail(`no rust-version is declared in ${manifestPath}`);
}

const rustcOutput = execFileSync('rustc', ['--version'], { encoding: 'utf8' }).trim();
const active = rustcOutput.match(/^rustc\s+(\S+)/)?.[1];

if (!active) {
  fail(`could not parse the active rustc version from: ${rustcOutput}`);
}

console.log(`src-tauri/Cargo.toml rust-version: ${declared}`);
console.log(`active rustc:                    ${active} (${rustcOutput})`);

if (active !== declared) {
  fail(
    `active rustc ${active} does not match the declared rust-version ${declared}. ` +
      'Install the declared toolchain before running the MSRV check ' +
      '(for example: rustup toolchain install ' +
      `${declared}).`,
  );
}

console.log('Rust toolchain matches the declared rust-version.');
