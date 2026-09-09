import { readdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const srcRoot = fileURLToPath(new URL('../', import.meta.url));
const svelteFiles = readdirSync(srcRoot, { recursive: true })
  .filter((path): path is string => typeof path === 'string' && path.endsWith('.svelte'));

function hslToRgb(hue: number, saturation: number, lightness: number): [number, number, number] {
  const s = saturation / 100;
  const l = lightness / 100;
  const chroma = (1 - Math.abs(2 * l - 1)) * s;
  const segment = hue / 60;
  const second = chroma * (1 - Math.abs((segment % 2) - 1));
  const [red, green, blue] = segment < 1
    ? [chroma, second, 0]
    : segment < 2
      ? [second, chroma, 0]
      : segment < 3
        ? [0, chroma, second]
        : segment < 4
          ? [0, second, chroma]
          : segment < 5
            ? [second, 0, chroma]
            : [chroma, 0, second];
  const match = l - chroma / 2;
  return [red + match, green + match, blue + match];
}

function relativeLuminance([red, green, blue]: [number, number, number]): number {
  const linearize = (channel: number) =>
    channel <= 0.03928 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  return 0.2126 * linearize(red) + 0.7152 * linearize(green) + 0.0722 * linearize(blue);
}

function contrastRatio(first: [number, number, number], second: [number, number, number]): number {
  const firstLuminance = relativeLuminance(first);
  const secondLuminance = relativeLuminance(second);
  const lighter = Math.max(firstLuminance, secondLuminance);
  const darker = Math.min(firstLuminance, secondLuminance);
  return (lighter + 0.05) / (darker + 0.05);
}

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

  it('uses the CSS-first Tailwind v4 entrypoint and Vite integration', () => {
    const css = readFileSync(`${srcRoot}/app.css`, 'utf8');
    const vite = readFileSync(`${srcRoot}/../vite.config.ts`, 'utf8');
    expect(css).toContain('@import "tailwindcss" source(none);');
    expect(css).toContain('@source "../index.html";');
    expect(css).toContain('@source "./**/*.{svelte,js,ts}";');
    expect(css).not.toContain('@tailwind base;');
    expect(css).not.toContain('@tailwind components;');
    expect(css).not.toContain('@tailwind utilities;');
    expect(vite).toContain("from '@tailwindcss/vite'");
    expect(vite).toContain('tailwindcss()');
  });

  it('defines a shared focus token for both light and dark themes', () => {
    const css = readFileSync(`${srcRoot}/app.css`, 'utf8');
    expect(css).toMatch(/--ring:\s*190\s+90%\s+35%/g);
    expect(css.match(/--ring:\s*190\s+90%\s+35%/g)).toHaveLength(2);
    expect(css).toContain('@custom-variant dark');

    const ring = hslToRgb(190, 90, 35);
    const surfaces = [
      hslToRgb(240, 10, 98), // light page background
      hslToRgb(0, 0, 100), // light card
      hslToRgb(240, 6, 10), // light primary control
      hslToRgb(240, 10, 7), // dark page background
      hslToRgb(240, 10, 10), // dark card
      hslToRgb(0, 0, 98), // dark primary control
    ];
    for (const surface of surfaces) {
      expect(contrastRatio(ring, surface)).toBeGreaterThanOrEqual(3);
    }
  });
});
