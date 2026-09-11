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

function readHslTokens(css: string, selector: ':root' | '.dark'): Map<string, [number, number, number]> {
  const escapedSelector = selector.replace('.', '\\.');
  const block = css.match(new RegExp(`${escapedSelector}\\s*\\{([\\s\\S]*?)\\}`))?.[1];
  if (!block) throw new Error(`Missing ${selector} theme block`);

  return new Map(
    Array.from(block.matchAll(/--([a-z-]+):\s*([\d.]+)\s+([\d.]+)%\s+([\d.]+)%/g), (match) => [
      match[1],
      [Number(match[2]), Number(match[3]), Number(match[4])] as [number, number, number],
    ])
  );
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

  it('keeps Windows and Korean font fallbacks with a stable scrollbar gutter', () => {
    const css = readFileSync(`${srcRoot}/app.css`, 'utf8');
    const sansStack = css.match(/--font-sans:([^;]+);/)?.[1] ?? '';
    const monoStack = css.match(/--font-mono:([^;]+);/)?.[1] ?? '';

    expect(sansStack).toContain('"Segoe UI"');
    for (const koreanFallback of ['"Apple SD Gothic Neo"', '"Malgun Gothic"', '"Noto Sans KR"']) {
      expect(sansStack).toContain(koreanFallback);
      expect(monoStack).toContain(koreanFallback);
    }
    // WebView2 classic scrollbars consume layout width; without a reserved
    // gutter a scrolling column shifts by the scrollbar width on Windows.
    expect(css).toContain('scrollbar-gutter: stable');
  });

  it('defines a shared focus token for both light and dark themes', () => {
    const css = readFileSync(`${srcRoot}/app.css`, 'utf8');
    expect(css).toContain('@custom-variant dark');

    for (const selector of [':root', '.dark'] as const) {
      const tokens = readHslTokens(css, selector);
      const ringToken = tokens.get('ring');
      if (!ringToken) throw new Error(`Missing ${selector} --ring token`);
      const ring = hslToRgb(...ringToken);

      for (const surfaceName of ['background', 'card', 'primary']) {
        const surfaceToken = tokens.get(surfaceName);
        if (!surfaceToken) throw new Error(`Missing ${selector} --${surfaceName} token`);
        expect(
          contrastRatio(ring, hslToRgb(...surfaceToken)),
          `${selector} --ring must contrast with --${surfaceName}`
        ).toBeGreaterThanOrEqual(3);
      }
    }
  });
});
