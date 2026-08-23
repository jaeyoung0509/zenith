import { readdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const srcRoot = fileURLToPath(new URL('../', import.meta.url));
const svelteFiles = readdirSync(srcRoot, { recursive: true })
  .filter((path): path is string => typeof path === 'string' && path.endsWith('.svelte'));

describe('design-system source contracts', () => {
  it('uses semantic colors instead of raw status palette utilities', () => {
    const violations = svelteFiles.filter((path) =>
      /(?:emerald|amber|rose|red)-(?:300|400|500|600)/.test(
        readFileSync(`${srcRoot}/${path}`, 'utf8')
      )
    );

    expect(violations).toEqual([]);
  });

  it('uses named micro type steps instead of arbitrary 9–11px utilities', () => {
    const violations = svelteFiles.filter((path) =>
      /text-\[(?:9|10|11)px\]/.test(readFileSync(`${srcRoot}/${path}`, 'utf8'))
    );

    expect(violations).toEqual([]);
  });
});
