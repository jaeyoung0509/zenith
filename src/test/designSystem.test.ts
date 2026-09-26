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

  it('keeps the brand on cool pastel neutrals and cobalt while success stays semantic', () => {
    const css = readFileSync(`${srcRoot}/app.css`, 'utf8');
    const light = readHslTokens(css, ':root');
    const background = light.get('background');
    if (!background) throw new Error('Missing light --background token');
    expect(background[0]).toBeGreaterThanOrEqual(220);
    expect(background[0]).toBeLessThanOrEqual(250);

    for (const selector of [':root', '.dark'] as const) {
      const tokens = readHslTokens(css, selector);
      for (const brandRole of ['primary', 'ai']) {
        const token = tokens.get(brandRole);
        if (!token) throw new Error(`Missing ${selector} --${brandRole} token`);
        expect(token[0], `${selector} --${brandRole} must stay in the cobalt family`)
          .toBeGreaterThanOrEqual(210);
        expect(token[0], `${selector} --${brandRole} must stay in the cobalt family`)
          .toBeLessThanOrEqual(230);
      }
      const success = tokens.get('success');
      if (!success) throw new Error(`Missing ${selector} --success token`);
      expect(success[0]).toBeGreaterThan(100);
      expect(success[0]).toBeLessThan(180);
    }
  });

  it('meets role-based text, action, and focus contrast in both themes', () => {
    const css = readFileSync(`${srcRoot}/app.css`, 'utf8');
    expect(css).toContain('@custom-variant dark');

    for (const selector of [':root', '.dark'] as const) {
      const tokens = readHslTokens(css, selector);
      const contrastFor = (foregroundName: string, surfaceName: string) => {
        const foregroundToken = tokens.get(foregroundName);
        const surfaceToken = tokens.get(surfaceName);
        if (!foregroundToken) throw new Error(`Missing ${selector} --${foregroundName} token`);
        if (!surfaceToken) throw new Error(`Missing ${selector} --${surfaceName} token`);
        return contrastRatio(hslToRgb(...foregroundToken), hslToRgb(...surfaceToken));
      };

      for (const surfaceName of ['background', 'card']) {
        expect(contrastFor('foreground', surfaceName)).toBeGreaterThanOrEqual(4.5);
        expect(contrastFor('muted-foreground', surfaceName)).toBeGreaterThanOrEqual(4.5);
        expect(contrastFor('ring', surfaceName)).toBeGreaterThanOrEqual(3);
      }
      expect(contrastFor('primary-foreground', 'primary')).toBeGreaterThanOrEqual(4.5);
      expect(contrastFor('action-foreground', 'action')).toBeGreaterThanOrEqual(4.5);
      expect(contrastFor('ai', 'card')).toBeGreaterThanOrEqual(4.5);
      expect(contrastFor('border-strong', 'card')).toBeGreaterThanOrEqual(3);
    }
  });

  it('keeps frosted panel text readable over black and white backdrops', () => {
    const css = readFileSync(`${srcRoot}/app.css`, 'utf8');
    for (const selector of [':root', '.dark'] as const) {
      const tokens = readHslTokens(css, selector);
      const block = css.match(new RegExp(`${selector.replace('.', '\\.')}\\s*\\{([\\s\\S]*?)\\}`))?.[1] ?? '';
      const opacity = Number(block.match(/--glass-opacity:\s*([\d.]+)/)?.[1]);
      expect(opacity).toBeGreaterThan(0);
      expect(opacity).toBeLessThan(1);
      const surface = hslToRgb(...tokens.get('glass-surface')!);
      for (const backdrop of [0, 1]) {
        const composite = surface.map(channel => channel * opacity + backdrop * (1 - opacity)) as [number, number, number];
        for (const role of ['foreground', 'glass-muted']) {
          expect(contrastRatio(hslToRgb(...tokens.get(role)!), composite), `${selector} ${role} over ${backdrop}`).toBeGreaterThanOrEqual(4.5);
        }
      }
    }
  });

  it('preserves the approved native tint and reduced-transparency override for both windows', () => {
    const css = readFileSync(`${srcRoot}/app.css`, 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
    const rules = Array.from(css.matchAll(/([^{}]+)\{([^{}]*)\}/g), match => ({
      selectors: match[1].trim().split(',').map(selector => selector.trim()),
      declarations: match[2],
    }));
    for (const selector of [
      ".native-liquid-glass .quick-liquid-shell[data-material='native']",
      '.native-liquid-glass.native-material-window .liquid-sidebar',
    ]) {
      const matching = rules.filter(rule => rule.selectors.includes(selector));
      expect(matching[0]?.declarations).toContain('background-color: hsl(var(--glass-surface) / 0.18)');
      expect(matching.at(-1)?.declarations).toContain('background: hsl(var(--secondary))');
    }
    for (const selector of [
      ".native-liquid-glass.dark .quick-liquid-shell[data-material='native']",
      '.native-liquid-glass.native-material-window.dark .liquid-sidebar',
    ]) {
      const matching = rules.filter(rule => rule.selectors.includes(selector));
      expect(matching[0]?.declarations).toContain('background-color: hsl(var(--glass-surface) / 0.55)');
      expect(matching.at(-1)?.declarations).toContain('background: hsl(var(--secondary))');
    }
  });
});
