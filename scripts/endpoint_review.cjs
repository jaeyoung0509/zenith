#!/usr/bin/env node

/**
 * Endpoint-protection review gate for Zenith releases.
 *
 * A release must be submitted to Microsoft's endpoint-protection analysis
 * (Defender / SmartScreen submission portal) before publication, and a
 * detection blocks publication. The analysis is not something this repository
 * can run automatically, so the maintainer records its outcome in the
 * `release-approval` environment and the release workflow turns that record
 * into a machine-checkable artifact bound to the exact published bytes:
 *
 *   record  - writes endpoint-review.json from the reviewed outcome plus the
 *             sha256 of every artifact about to be published. Exits 1 when the
 *             recorded outcome is not "clear".
 *   verify  - re-reads endpoint-review.json and fails unless every artifact
 *             hash still matches the bytes being published, the version and
 *             commit match, and the recorded outcome is "clear".
 *
 * `verify` is what blocks publication. A missing, unreadable, incomplete, or
 * mismatched record fails closed.
 *
 * Usage:
 *   node scripts/endpoint_review.cjs record \
 *     --output artifacts/endpoint-review.json \
 *     --version 0.3.18 --commit <sha> \
 *     --status clear --reference <portal-submission-id> \
 *     --reviewer <github-login> --reviewed-at 2026-09-12 \
 *     artifacts/Zenith-macos-arm64.dmg artifacts/Zenith-windows-x64-setup.exe
 *
 *   node scripts/endpoint_review.cjs verify \
 *     --review artifacts/endpoint-review.json \
 *     --version 0.3.18 --commit <sha> \
 *     artifacts/Zenith-macos-arm64.dmg artifacts/Zenith-windows-x64-setup.exe
 */

const crypto = require('crypto');
const fs = require('fs');
const path = require('path');

const SCHEMA_VERSION = 1;
const CLEAR = 'clear';
const DETECTED = 'detected';
const KNOWN_STATUSES = [CLEAR, DETECTED];

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

function requireField(options, field) {
  const value = options[field];
  if (value === undefined || value.trim() === '') {
    fail(`--${field} is required`);
  }
  return value.trim();
}

function requireArtifact(artifactPath) {
  const resolved = path.resolve(artifactPath);
  const stats = fs.statSync(resolved, { throwIfNoEntry: false });
  if (!stats?.isFile()) {
    fail(`release artifact does not exist: ${resolved}`);
  }
  return { path: resolved, name: path.basename(resolved) };
}

function sha256(filePath) {
  return crypto.createHash('sha256').update(fs.readFileSync(filePath)).digest('hex');
}

function describeArtifacts(paths) {
  if (paths.length === 0) {
    fail('at least one release artifact path is required');
  }

  const described = paths.map(requireArtifact).map(({ path: artifactPath, name }) => ({
    name,
    sha256: sha256(artifactPath),
    bytes: fs.statSync(artifactPath).size,
  }));

  const names = new Set();
  for (const artifact of described) {
    if (names.has(artifact.name)) {
      fail(`duplicate artifact name: ${artifact.name}`);
    }
    names.add(artifact.name);
  }

  return described.sort((left, right) => left.name.localeCompare(right.name));
}

function requireClearStatus(status) {
  if (!KNOWN_STATUSES.includes(status)) {
    fail(`--status must be one of: ${KNOWN_STATUSES.join(', ')}`);
  }

  if (status !== CLEAR) {
    fail(
      `the recorded endpoint-protection result is "${status}"; publication is ` +
        'blocked until Microsoft reports no detection for these exact bytes.',
    );
  }
}

function writeJson(filePath, document) {
  const resolved = path.resolve(filePath);
  fs.mkdirSync(path.dirname(resolved), { recursive: true });
  // LF-only output on every runner; the release workflows run this on Linux,
  // macOS, and Windows.
  fs.writeFileSync(resolved, `${JSON.stringify(document, null, 2)}\n`, 'utf8');
  return resolved;
}

function record(options, positional) {
  const status = requireField(options, 'status');
  if (!KNOWN_STATUSES.includes(status)) {
    fail(`--status must be one of: ${KNOWN_STATUSES.join(', ')}`);
  }

  const artifacts = describeArtifacts(positional);
  const submission = {
    reviewer: requireField(options, 'reviewer'),
    reviewed_at: requireField(options, 'reviewed-at'),
    status,
    portal_reference: requireField(options, 'reference'),
  };

  if (status !== CLEAR) {
    console.error(
      `Endpoint-protection review ${submission.portal_reference} reported ` +
        `"${status}" for:`,
    );
    for (const artifact of artifacts) {
      console.error(`  - ${artifact.name} (sha256 ${artifact.sha256})`);
    }
    fail('publication is blocked by a detection.');
  }

  const review = {
    schema: SCHEMA_VERSION,
    generated_by: 'scripts/endpoint_review.cjs',
    version: requireField(options, 'version'),
    commit: requireField(options, 'commit'),
    submission,
    artifacts,
  };

  const outputPath = writeJson(requireField(options, 'output'), review);
  console.log(`Endpoint-protection review recorded at ${outputPath}:`);
  for (const artifact of review.artifacts) {
    console.log(`  - ${artifact.name} ${artifact.sha256}`);
  }
}

function verify(options, positional) {
  const reviewPath = path.resolve(requireField(options, 'review'));
  if (!fs.statSync(reviewPath, { throwIfNoEntry: false })?.isFile()) {
    fail(
      `endpoint-protection review record is missing: ${reviewPath}. The release ` +
        'is blocked until the submission result is recorded.',
    );
  }

  let review;
  try {
    review = JSON.parse(fs.readFileSync(reviewPath, 'utf8'));
  } catch (error) {
    fail(`endpoint-protection review record is not valid JSON: ${error.message}`);
  }

  if (review.schema !== SCHEMA_VERSION) {
    fail(`unexpected endpoint-review schema: ${review.schema}`);
  }

  const submission = review.submission ?? {};
  for (const field of ['reviewer', 'reviewed_at', 'status', 'portal_reference']) {
    if (typeof submission[field] !== 'string' || submission[field].trim() === '') {
      fail(`endpoint-protection review is missing submission.${field}`);
    }
  }
  requireClearStatus(submission.status);

  const version = requireField(options, 'version');
  const commit = requireField(options, 'commit');
  if (review.version !== version) {
    fail(`endpoint-protection review covers version ${review.version}, not ${version}`);
  }
  if (review.commit !== commit) {
    fail(`endpoint-protection review covers commit ${review.commit}, not ${commit}`);
  }

  const expected = Array.isArray(review.artifacts) ? review.artifacts : [];
  const actual = describeArtifacts(positional);
  const actualByName = new Map(actual.map((artifact) => [artifact.name, artifact]));

  for (const artifact of expected) {
    const current = actualByName.get(artifact.name);
    if (!current) {
      fail(`endpoint-protection review covers ${artifact.name}, which was not provided`);
    }
    if (current.sha256 !== artifact.sha256) {
      fail(
        `${artifact.name} does not match the reviewed bytes: reviewed ` +
          `${artifact.sha256}, publishing ${current.sha256}`,
      );
    }
  }

  if (expected.length !== actual.length) {
    fail(
      `endpoint-protection review covers ${expected.length} artifact(s) but ` +
        `${actual.length} were provided`,
    );
  }

  console.log(
    `Endpoint-protection review verified: ${submission.status} for ${actual.length} ` +
      `artifact(s), submission ${submission.portal_reference}, reviewed by ` +
      `${submission.reviewer} on ${submission.reviewed_at}.`,
  );
}

const command = process.argv[2];
const { options, positional } = parseOptions(process.argv.slice(3));

if (command === 'record') {
  record(options, positional);
} else if (command === 'verify') {
  verify(options, positional);
} else {
  fail('expected record or verify command');
}
