#!/usr/bin/env node

/**
 * Enforces the workspace dependency boundaries.
 *
 * `zenith-core` exists so Zenith's product semantics can outlive the desktop
 * framework: the crate must still compile and make sense if Zenith grew a CLI
 * or a second front end. That property dies quietly the first time a domain
 * module imports a webview, a window, or a Win32 binding, and a review comment
 * is not a mechanism.
 *
 * `zenith-platform` exists so every native OS integration has one owner: the
 * same probing, path resolution, process control, and Trash adapter has to be
 * usable by a scan, a service, or a future CLI without a window, which fails
 * the first time a platform module imports the framework.
 *
 * Both boundaries also name `zenith-desktop`, the crate that owns the window,
 * the framework, and the command surface. Dependencies in this workspace point
 * into the domain, never back out of it: `zenith-desktop` depends on
 * `zenith-core` and `zenith-platform`, so an edge from either crate up to it
 * inverts the layering and lets a domain or platform rule start depending on a
 * window. Reaching only the framework one hop later — `zenith-core` to a
 * helper crate to `zenith-desktop` — is the same coupling, which is why rule 2
 * below is transitive rather than a direct-edge check.
 *
 * So this check reads the resolved dependency graph from `cargo metadata` and
 * applies two rules to each crate that declares a boundary:
 *
 *   1. Every dependency the crate declares, of any kind, must be outside its
 *      forbidden set. Pulling `tauri` in as a dev-dependency "just for a
 *      test" recouples the crate's own test surface, so it is refused too.
 *
 *   2. Every crate reachable from it over normal and build edges must be
 *      outside the forbidden set, at any depth. Transitive reachability is the
 *      point: a direct-edge check would pass while
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

/**
 * The crates with a dependency boundary, and what each one may never reach.
 *
 * The `tauri` and `windows` rules are prefixes rather than exact names:
 * `tauri-build` and `tauri-plugin-*` are named in the boundary directly, and a
 * rule that listed them one by one would go stale the next time the framework
 * adds a crate. Every crate those prefixes match is a binding to the desktop
 * framework or to Win32.
 *
 * `zenith-platform` is the layer that *owns* the native bindings, so Win32 and
 * macOS frameworks are the point of the crate rather than a violation. What it
 * must never gain is the desktop framework: the reason the platform layer
 * exists is that the same native probing has to be usable by a scanner, a
 * service, or a future CLI without a window.
 *
 * `zenith-desktop` is matched exactly, not by prefix: it is a workspace member
 * that rule 1 and rule 2 both have to recognize by package name, and no
 * crates.io package shares it.
 */
const BOUNDARIES = [
  {
    crate: 'zenith-core',
    intent:
      'the domain must not depend on the desktop framework, on native platform bindings, or on the desktop adapter',
    forbidden: [
      { name: 'tauri', kind: 'prefix', why: 'desktop framework' },
      { name: 'windows', kind: 'prefix', why: 'Win32 bindings' },
      { name: 'security-framework', kind: 'prefix', why: 'macOS keychain bindings' },
      { name: 'rfd', kind: 'exact', why: 'native file dialogs' },
      {
        name: 'zenith-desktop',
        kind: 'exact',
        why: 'the outer adapter, which owns the window and the framework',
      },
    ],
    remedy:
      'Move the code that needs the binding or the window up to `zenith-desktop`, or express\n' +
      'what the domain needs as a trait in `zenith-core` and implement it in the adapter that\n' +
      'owns the binding. An edge from here to `zenith-desktop` is deleted rather than moved:\n' +
      'the desktop crate depends on this one, never the other way round.',
  },
  {
    crate: 'zenith-platform',
    intent:
      'the platform adapter layer must not depend on the desktop framework or on the desktop adapter',
    forbidden: [
      { name: 'tauri', kind: 'prefix', why: 'desktop framework' },
      {
        name: 'zenith-desktop',
        kind: 'exact',
        why: 'the outer adapter, which owns the window and the framework',
      },
    ],
    remedy:
      'Move the framework-facing part to `zenith-desktop` and keep the native call here,\n' +
      'behind a port the adapter implements. `zenith-desktop` already depends on this crate,\n' +
      'so an edge back to it inverts the layering and is removed, not relocated.',
  },
];

function fail(message) {
  console.error(`Error: ${message}`);
  process.exit(1);
}

function forbiddenReason(forbidden, crateName) {
  for (const rule of forbidden) {
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

  if (!metadata.resolve) {
    fail('cargo metadata did not include a resolve graph; run it without --no-deps');
  }

  for (const boundary of BOUNDARIES) {
    const crate = metadata.packages.find((pkg) => pkg.name === boundary.crate);

    if (!crate) {
      fail(`no workspace member named \`${boundary.crate}\` was found`);
    }
    if (!members.has(crate.id)) {
      fail(`\`${boundary.crate}\` exists but is not a workspace member`);
    }

    const violations = [];

    // Rule 1: what the crate declares, of any kind.
    for (const dependency of crate.dependencies ?? []) {
      const why = forbiddenReason(boundary.forbidden, dependency.name);
      if (why) {
        violations.push({
          label: `${dependency.name} (${dependency.req}) — declared as a ${dependency.kind ?? 'normal'} dependency`,
          why,
        });
      }
    }

    // Rule 2: what the library actually links, transitively.
    const reached = reachableFrom(metadata, crate.id);
    for (const [id, introducedBy] of reached) {
      const pkg = metadata.packages.find((candidate) => candidate.id === id);
      if (!pkg) {
        continue;
      }
      const why = forbiddenReason(boundary.forbidden, pkg.name);
      if (why) {
        const via = introducedBy ? ` (via ${introducedBy})` : '';
        violations.push({ label: `${pkg.name} ${pkg.version}${via}`, why });
      }
    }

    violations.sort((left, right) => left.label.localeCompare(right.label));

    if (violations.length > 0) {
      console.error(`\`${boundary.crate}\` ${boundary.intent}.`);
      console.error(
        `Reached ${reached.size} crates over runtime edges; ${violations.length} violations:\n`,
      );
      for (const violation of violations) {
        console.error(`  ✗ ${violation.label} — ${violation.why}`);
      }
      console.error(`\n${boundary.remedy}`);
      process.exit(1);
    }

    console.log(
      `✅ ${boundary.crate} declares no forbidden dependency and reaches ${reached.size} crates over runtime edges without touching one.`,
    );
  }
}

main();
