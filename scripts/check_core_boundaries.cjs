#!/usr/bin/env node

/**
 * Enforces the zenith-core dependency boundary.
 *
 * `zenith-core` exists so Zenith's product semantics can outlive the desktop
 * framework: the crate must still compile and make sense if Zenith grew a CLI
 * or a second front end. That property dies quietly the first time a domain
 * module imports a webview, a window, or a Win32 binding, and a review comment
 * is not a mechanism.
 *
 * So this check reads the resolved dependency graph from `cargo metadata` and
 * applies two rules:
 *
 *   1. Every dependency `zenith-core` declares, of any kind, must be outside
 *      the forbidden set. Pulling `tauri` in as a dev-dependency "just for a
 *      test" recouples the crate's own test surface, so it is refused too.
 *
 *   2. Every crate reachable from `zenith-core` over normal and build edges
 *      must be outside the forbidden set, at any depth. Transitive
 *      reachability is the point: a direct-edge check would pass while
 *      `zenith-core -> some-helper -> tauri` rebuilt the coupling one layer
 *      down. Dev-only edges are excluded here because they describe what the
 *      test harness links, not what the library is; `tempfile` reaching
 *      `windows-sys` for its own implementation is not `zenith-core` depending
 *      on Win32.
 *
 * Usage:
 *   node scripts/check_core_boundaries.cjs
 *   node scripts/check_core_boundaries.cjs --metadata <captured-metadata.json>
 */

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const rootDir = path.resolve(__dirname, '..');
const CORE_CRATE = 'zenith-core';

/**
 * Crates the domain must never reach.
 *
 * The `tauri` and `windows` rules are prefixes rather than exact names:
 * `tauri-build` and `tauri-plugin-*` are named in the boundary directly, and a
 * rule that listed them one by one would go stale the next time the framework
 * adds a crate. Every crate those prefixes match is a binding to the desktop
 * framework or to Win32.
 */
const FORBIDDEN = [
  { name: 'tauri', kind: 'prefix', why: 'desktop framework' },
  { name: 'windows', kind: 'prefix', why: 'Win32 bindings' },
  { name: 'security-framework', kind: 'prefix', why: 'macOS keychain bindings' },
  { name: 'rfd', kind: 'exact', why: 'native file dialogs' },
];

function fail(message) {
  console.error(`Error: ${message}`);
  process.exit(1);
}

function forbiddenReason(crateName) {
  for (const rule of FORBIDDEN) {
    const matches =
      rule.kind === 'exact' ? crateName === rule.name : crateName.startsWith(rule.name);
    if (matches) {
      return rule.why;
    }
  }
  return null;
}

function readMetadata(argv) {
  const flagIndex = argv.indexOf('--metadata');
  if (flagIndex !== -1) {
    const file = argv[flagIndex + 1];
    if (!file) {
      fail('--metadata needs a path to a captured `cargo metadata` document');
    }
    return JSON.parse(fs.readFileSync(file, 'utf8'));
  }

  // `--locked` keeps the check from rewriting Cargo.lock as a side effect of
  // asking a question about it.
  const output = execFileSync(
    'cargo',
    ['metadata', '--format-version', '1', '--locked'],
    { cwd: rootDir, encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 },
  );
  return JSON.parse(output);
}

/** The packages `node` pulls in over edges that build the library. */
function runtimeEdges(node) {
  if (!node.deps) {
    // Pre-1.77 documents list only normal dependency ids.
    return node.dependencies ?? [];
  }
  const targets = [];
  for (const dep of node.deps) {
    const kinds = dep.dep_kinds ?? [];
    if (kinds.length === 0 || kinds.some((kind) => kind.kind !== 'dev')) {
      targets.push(dep.pkg);
    }
  }
  return targets;
}

/**
 * Every package reachable from `rootId` over runtime edges.
 *
 * Returns a map of package id -> the name of the crate that first pulled it
 * in (or `null` for the root itself), so a violation is reported with the
 * edge that created it rather than with the crate name alone.
 */
function reachableFrom(metadata, rootId) {
  const byId = new Map(metadata.packages.map((pkg) => [pkg.id, pkg]));
  const edges = new Map(
    (metadata.resolve?.nodes ?? []).map((node) => [node.id, runtimeEdges(node)]),
  );

  const reached = new Map([[rootId, null]]);
  const queue = [rootId];
  while (queue.length > 0) {
    const current = queue.shift();
    for (const next of edges.get(current) ?? []) {
      if (reached.has(next)) {
        continue;
      }
      reached.set(next, byId.get(current)?.name ?? current);
      queue.push(next);
    }
  }
  return reached;
}

function main() {
  const metadata = readMetadata(process.argv.slice(2));
  const members = new Set(metadata.workspace_members ?? []);
  const core = metadata.packages.find((pkg) => pkg.name === CORE_CRATE);

  if (!core) {
    fail(`no workspace member named \`${CORE_CRATE}\` was found`);
  }
  if (!members.has(core.id)) {
    fail(`\`${CORE_CRATE}\` exists but is not a workspace member`);
  }
  if (!metadata.resolve) {
    fail('cargo metadata did not include a resolve graph; run it without --no-deps');
  }

  const violations = [];

  // Rule 1: what the crate declares, of any kind.
  for (const dependency of core.dependencies ?? []) {
    const why = forbiddenReason(dependency.name);
    if (why) {
      violations.push({
        label: `${dependency.name} (${dependency.req}) — declared as a ${dependency.kind ?? 'normal'} dependency`,
        why,
      });
    }
  }

  // Rule 2: what the library actually links, transitively.
  const reached = reachableFrom(metadata, core.id);
  for (const [id, introducedBy] of reached) {
    const pkg = metadata.packages.find((candidate) => candidate.id === id);
    if (!pkg) {
      continue;
    }
    const why = forbiddenReason(pkg.name);
    if (why) {
      const via = introducedBy ? ` (via ${introducedBy})` : '';
      violations.push({ label: `${pkg.name} ${pkg.version}${via}`, why });
    }
  }

  violations.sort((left, right) => left.label.localeCompare(right.label));

  if (violations.length > 0) {
    console.error(
      `\`${CORE_CRATE}\` must not depend on the desktop framework or on native platform bindings.`,
    );
    console.error(`Reached ${reached.size} crates over runtime edges; ${violations.length} violations:\n`);
    for (const violation of violations) {
      console.error(`  ✗ ${violation.label} — ${violation.why}`);
    }
    console.error(
      '\nMove the dependency to `zenith-desktop`, or express what the domain needs as a\n' +
        'trait in `zenith-core` and implement it in the adapter that owns the binding.',
    );
    process.exit(1);
  }

  console.log(
    `✅ ${CORE_CRATE} declares no forbidden dependency and reaches ${reached.size} crates over runtime edges without touching one.`,
  );
}

main();
