#!/usr/bin/env node
/** Generate Zenith's own icon family. Third-party brand assets are not inputs. */
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, mkdirSync, readdirSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const check = process.argv.includes('--check');
const source = readFileSync(join(root, 'src-tauri/icons/zenith-mark.svg'), 'utf8');
const artwork = source.match(/<svg\b[^>]*>([\s\S]*)<\/svg>/)?.[1]?.trim();
const paths = [...source.matchAll(/<path d="([^"]+)"/g)].map(match => match[1]);
if (!artwork || paths.length !== 3) throw new Error('The master ribbon must contain three filled paths.');

// The approved ribbon master owns its gradients; the generator owns the tile.
const palette = { paper: '#F5F8FE', edge: '#D6E3F8', highlight: '#FDFEFF', shade: '#E8F1FF', shadow: '#212636' };
const svg = (size, content, viewBox = `0 0 ${size} ${size}`) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="${size}" height="${size}" viewBox="${viewBox}">\n${content}\n</svg>\n`;
const mark = (transform = '') => `<g${transform ? ` transform="${transform}"` : ''}>\n${artwork}\n</g>`;
const templateMark = paths.map(path => `  <path d="${path}" fill="#000"/>`).join('\n');
const app = svg(1024, `  <defs>
    <linearGradient id="tile" x1="160" y1="96" x2="800" y2="928" gradientUnits="userSpaceOnUse">
      <stop stop-color="${palette.highlight}"/>
      <stop offset="1" stop-color="${palette.shade}"/>
    </linearGradient>
    <filter id="shadow" x="32" y="48" width="960" height="944" filterUnits="userSpaceOnUse">
      <feDropShadow dx="0" dy="16" stdDeviation="16" flood-color="${palette.shadow}" flood-opacity=".16"/>
    </filter>
  </defs>
  <rect x="96" y="96" width="832" height="832" rx="188" fill="url(#tile)" stroke="${palette.edge}" stroke-width="2" filter="url(#shadow)"/>
${mark('translate(96 85) scale(13)')}`);
// Compact UI artwork omits the app bundle's outer padding and drop shadow.
const compact = svg(64, `  <rect x="1" y="1" width="62" height="62" rx="15" fill="${palette.paper}"/>
${mark()}`);
const tray = svg(44, templateMark, '4 4 56 56');
const temp = mkdtempSync(join(tmpdir(), 'zenith-icons-'));
const generated = new Map();
const add = (relative, content) => generated.set(relative, Buffer.from(content));

// The ICNS encoder emits its size records from an unordered map. Sort records
// without changing their image payloads so regeneration has stable bytes.
function canonicalIcns(bytes) {
  const records = [];
  for (let offset = 8; offset < bytes.length;) {
    const length = bytes.readUInt32BE(offset + 4);
    if (length < 8 || offset + length > bytes.length) throw new Error('Invalid ICNS record');
    records.push(bytes.subarray(offset, offset + length));
    offset += length;
  }
  records.sort((first, second) => Buffer.compare(first.subarray(0, 4), second.subarray(0, 4)));
  return Buffer.concat([bytes.subarray(0, 8), ...records]);
}

function runIcon(input, output, sizes = []) {
  const cli = resolve(root, 'node_modules/@tauri-apps/cli/tauri.js');
  const args = [cli, 'icon', input, '--output', output, ...sizes.flatMap(size => ['--png', String(size)])];
  const result = spawnSync(process.execPath, args, { cwd: root, encoding: 'utf8' });
  if (result.status !== 0) throw new Error(result.stderr || result.stdout || 'Tauri icon generation failed');
}

try {
  const appSource = join(temp, 'app.svg');
  const compactSource = join(temp, 'compact.svg');
  const traySource = join(temp, 'tray.svg');
  writeFileSync(appSource, app);
  writeFileSync(compactSource, compact);
  writeFileSync(traySource, tray);
  runIcon(appSource, join(temp, 'bundle'));
  runIcon(compactSource, join(temp, 'web'), [32, 512]);
  runIcon(traySource, join(temp, 'tray'), [44]);
  for (const file of readdirSync(join(temp, 'bundle'), { recursive: true })) {
    if (!/\.(png|ico|icns|xml)$/.test(file)) continue;
    const bytes = readFileSync(join(temp, 'bundle', file));
    generated.set(`src-tauri/icons/${file}`, file.endsWith('.icns') ? canonicalIcns(bytes) : bytes);
  }
  add('src-tauri/icons/app-icon.svg', app);
  add('src-tauri/icons/tray-icon.svg', tray);
  add('public/app-icon.svg', app);
  add('public/favicon.svg', compact);
  add('src/lib/assets/brands/zenith.svg', compact);
  generated.set('src-tauri/icons/tray-icon.png', readFileSync(join(temp, 'tray/44x44.png')));
  // Explicitly regenerate the historical 64 px desktop asset as well.
  runIcon(appSource, join(temp, 'desktop'), [64]);
  generated.set('src-tauri/icons/64x64.png', readFileSync(join(temp, 'desktop/64x64.png')));
  generated.set('public/favicon.png', readFileSync(join(temp, 'web/32x32.png')));
  generated.set('public/icon.png', readFileSync(join(temp, 'web/512x512.png')));
  const registryPath = 'src/lib/utils/brandIcons.ts';
  const registry = readFileSync(join(root, registryPath), 'utf8');
  const hash = createHash('sha256').update(compact).digest('hex');
  const updated = registry.replace(/(file: 'zenith\.svg',[\s\S]*?sha256: ')[a-f0-9]{64}(')/, `$1${hash}$2`);
  if (updated === registry && !registry.includes(hash)) throw new Error('Zenith registry entry not found');
  add(registryPath, updated);
  const drift = [];
  for (const [relative, bytes] of generated) {
    const target = join(root, relative);
    if (check) {
      if (!existsSync(target) || !readFileSync(target).equals(bytes)) drift.push(relative);
    } else {
      mkdirSync(dirname(target), { recursive: true });
      writeFileSync(target, bytes);
    }
  }
  if (drift.length) throw new Error(`Generated icon drift:\n${drift.join('\n')}\nRun pnpm icons:generate.`);
  console.log(`${check ? 'Verified' : 'Generated'} ${generated.size} Zenith icon assets and registry entries.`);
} finally {
  rmSync(temp, { recursive: true, force: true });
}
