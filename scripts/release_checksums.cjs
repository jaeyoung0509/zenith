#!/usr/bin/env node

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

function fail(message) {
  console.error(`Error: ${message}`);
  process.exit(1);
}

function parseOptions(argv) {
  const options = {};
  const positional = [];

  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (!argument.startsWith('--')) {
      positional.push(argument);
      continue;
    }

    const value = argv[index + 1];
    if (value === undefined || value.startsWith('--')) {
      fail(`missing value for ${argument}`);
    }
    options[argument.slice(2)] = value;
    index += 1;
  }

  return { options, positional };
}

function requireFile(filePath, label) {
  const resolvedPath = path.resolve(filePath);
  if (!fs.statSync(resolvedPath, { throwIfNoEntry: false })?.isFile()) {
    fail(`${label} does not exist: ${resolvedPath}`);
  }
  return resolvedPath;
}

function normalizeManifest(contents, sourcePath) {
  const normalized = contents.replace(/\r\n?/g, '\n').trimEnd();
  if (!normalized) {
    fail(`checksum manifest is empty: ${sourcePath}`);
  }

  for (const line of normalized.split('\n')) {
    if (!/^[0-9a-fA-F]{64}  [^\r\n]+$/.test(line)) {
      fail(`invalid checksum entry in ${sourcePath}: ${line}`);
    }
  }

  return `${normalized}\n`;
}

function writeChecksum(options, positional) {
  if (positional.length > 0 || !options.file || !options.output) {
    fail('write expects --file and --output arguments');
  }

  const filePath = requireFile(options.file, 'release artifact');
  const outputPath = path.resolve(options.output);
  const hash = crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex');
  fs.writeFileSync(outputPath, `${hash}  ${path.basename(filePath)}\n`, 'ascii');
}

function combineChecksums(options, positional) {
  if (!options.output || positional.length === 0) {
    fail('combine expects --output followed by one or more checksum manifests');
  }

  const outputPath = path.resolve(options.output);
  const combined = positional
    .map((manifestPath) => {
      const sourcePath = requireFile(manifestPath, 'checksum manifest');
      return normalizeManifest(fs.readFileSync(sourcePath, 'utf8'), sourcePath);
    })
    .join('');
  fs.writeFileSync(outputPath, combined, 'ascii');
}

const command = process.argv[2];
const { options, positional } = parseOptions(process.argv.slice(3));

if (command === 'write') {
  writeChecksum(options, positional);
} else if (command === 'combine') {
  combineChecksums(options, positional);
} else {
  fail('expected write or combine command');
}
