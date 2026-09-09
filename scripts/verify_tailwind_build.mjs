import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const assetsDir = join(process.cwd(), 'dist', 'assets');
const cssFiles = readdirSync(assetsDir).filter((file) => file.endsWith('.css'));
if (cssFiles.length === 0) {
  throw new Error('Tailwind verification could not find a generated CSS asset');
}

const css = cssFiles.map((file) => readFileSync(join(assetsDir, file), 'utf8')).join('\n');
const required = [
  '.duration-140',
  '.shadow-xs',
  '.focus-visible\\:ring-2',
  '.bg-background',
  '.dark\\:',
  '--background:',
  '--ring:',
];
const missing = required.filter((marker) => !css.includes(marker));
if (missing.length > 0) {
  throw new Error(`Tailwind production CSS is missing: ${missing.join(', ')}`);
}

console.log(`Verified Tailwind utilities in ${cssFiles.length} production CSS asset(s).`);
