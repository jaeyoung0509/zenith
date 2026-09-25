import { createHash } from 'node:crypto';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import BrandIcon from '../lib/components/BrandIcon.svelte';
import {
  BRAND_IDENTITIES,
  brandFallbackGlyphs,
  brandIconProps,
  resolveBrandIdentity,
  type BrandIdentity,
  type BrandIdentityRecord,
} from '../lib/utils/brandIcons';

const srcRoot = fileURLToPath(new URL('../', import.meta.url));
const brandsDir = fileURLToPath(new URL('../lib/assets/brands/', import.meta.url));
const registry = Object.entries(BRAND_IDENTITIES) as Array<[BrandIdentity, BrandIdentityRecord]>;
const bundled = registry.filter(([, record]) => record.asset !== null);
const unresolved = registry.filter(([, record]) => record.asset === null);

/**
 * SVG files must declare the SVG and xlink namespaces, and those declarations
 * are the only permitted `http://` spelling in a reviewed asset. Everything
 * else — a remote reference, an embedded script, a data-URI raster, an @import
 * — is what the scan below rejects.
 */
function stripW3cNamespaceDeclarations(svg: string): string {
  return svg
    .split('xmlns="http://www.w3.org/2000/svg"')
    .join('')
    .split('xmlns:xlink="http://www.w3.org/1999/xlink"')
    .join('')
    .split('xmlns="http://www.w3.org/1999/xlink"')
    .join('');
}

function renderBrandIcon(identity: string | null, label: string, size?: 20 | 24 | 32): string {
  return render(BrandIcon, { props: { identity, label, ...(size === undefined ? {} : { size }) } })
    .body;
}

/**
 * Reads the artwork size out of rendered markup: a bundled asset pins its
 * height in the inline style, a Lucide glyph carries it as an attribute.
 */
function drawnArtworkPx(body: string): number {
  const styled = /height: (\d+)px; width: auto/.exec(body)?.[1];
  const attributed = styled ?? /height="(\d+)"/.exec(body)?.[1];
  return Number(attributed ?? '0');
}

describe('brand identity registry', () => {
  it('bundles only reviewed files that exist on disk byte-for-byte', () => {
    expect(bundled.length).toBeGreaterThan(0);

    for (const [identity, record] of bundled) {
      const asset = record.asset;
      if (asset === null) throw new Error(`${identity} is not bundled`);

      expect(asset.src, `${identity} must bundle a local asset, never a remote one`).not.toMatch(
        /^(?:https?:)?\/\//
      );
      expect(asset.src.startsWith('/'), `${identity} asset must be a local URL`).toBe(true);
      expect(asset.src, `${identity} must bundle the reviewed file`).toContain(
        `/brands/${asset.file}`
      );

      const bytes = readFileSync(join(brandsDir, asset.file));
      expect(bytes.byteLength, `${identity} asset is empty`).toBeGreaterThan(0);
      expect(
        createHash('sha256').update(bytes).digest('hex'),
        `${identity} asset drifted from the reviewed bytes`
      ).toBe(asset.sha256);
      expect(record.unresolvedReason, `${identity} is bundled and cannot carry a reason`).toBeNull();
    }
  });

  it('keeps the assets directory identical to the reviewed registry', () => {
    const registered = bundled.map(([, record]) => record.asset?.file ?? '').sort();
    expect(readdirSync(brandsDir).sort()).toEqual(registered);
  });

  it('gives every unresolved identity a fallback glyph and no asset', () => {
    expect(unresolved.length).toBeGreaterThan(0);

    for (const [identity, record] of unresolved) {
      expect(record.asset, `${identity} must not bundle an asset`).toBeNull();
      expect(record.unresolvedReason ?? '', `${identity} needs a recorded reason`).not.toBe('');

      const body = renderBrandIcon(identity, record.label);
      expect(body, `${identity} must render its fallback glyph`).toContain('<svg');
      expect(body, `${identity} must not render an image`).not.toContain('<img');
      expect(brandFallbackGlyphs, `${identity} glyph is unmapped`).toHaveProperty(
        record.fallbackGlyph
      );
      expect(body, `${identity} fallback stays decorative`).toContain('aria-hidden="true"');
    }
  });

  it('renders every bundled identity as decorative artwork', () => {
    for (const [identity, record] of bundled) {
      const body = renderBrandIcon(identity, record.label);

      expect(body, `${identity} must render an image`).toContain('<img');
      expect(body, `${identity} must stay decorative`).toContain('alt=""');
      expect(body, `${identity} must be hidden from assistive tech`).toContain('aria-hidden="true"');
      expect(body, `${identity} must not double up with a second glyph`).not.toContain('<svg');
    }
  });

  it('renders the default 32 px slot with 24 px artwork', () => {
    const body = renderBrandIcon('vite', 'Vite');

    expect(body, 'the identity slot').toContain('width: 32px; height: 32px;');
    expect(body, 'the artwork inside it').toContain('height: 24px; width: auto;');
  });

  it('fills a smaller slot but never draws artwork below the brand minimum', () => {
    for (const [identity, record] of registry) {
      const artworkPx = drawnArtworkPx(renderBrandIcon(identity, record.label, 20));

      expect(artworkPx, `${identity} at a 20 px slot`).toBeGreaterThanOrEqual(record.minSizePx);
    }

    expect(renderBrandIcon('vite', 'Vite', 20)).toContain('height: 20px; width: auto;');
    expect(renderBrandIcon('docker', 'Docker', 20)).toContain('width: 24px; height: 24px;');
    expect(renderBrandIcon('docker', 'Docker', 20)).toContain('width="24" height="24"');
  });
});

describe('brand identity resolution', () => {
  it('resolves canonical ids whether they are typed in any case', () => {
    for (const [identity] of registry) {
      expect(resolveBrandIdentity(identity)).toBe(identity);
      expect(resolveBrandIdentity(identity.toUpperCase())).toBe(identity);
      expect(resolveBrandIdentity(` ${identity} `)).toBe(identity);
    }
  });

  it('resolves aliases the app and datasets use', () => {
    for (const [identity, record] of registry) {
      for (const alias of record.aliases) {
        expect(resolveBrandIdentity(alias), `${alias} must reach ${identity}`).toBe(identity);
      }
    }

    expect(resolveBrandIdentity('gro k-build')).toBeNull();
    expect(resolveBrandIdentity('Grok_Build')).toBe('grok');
    expect(resolveBrandIdentity('xai-api')).toBe('grok');
    expect(resolveBrandIdentity('anthropic-api')).toBe('claude');
    expect(resolveBrandIdentity('openai-api')).toBe('codex');
    expect(resolveBrandIdentity('GitHub  Copilot')).toBe('copilot');
    expect(resolveBrandIdentity('visual studio code')).toBe('vscode');
    expect(resolveBrandIdentity('lm_studio')).toBe('lmstudio');
  });

  it('returns null for identities outside the reviewed registry', () => {
    expect(resolveBrandIdentity('')).toBeNull();
    expect(resolveBrandIdentity('   ')).toBeNull();
    expect(resolveBrandIdentity(null)).toBeNull();
    expect(resolveBrandIdentity(undefined)).toBeNull();
    expect(resolveBrandIdentity('definitely-not-a-brand')).toBeNull();
    // Provider ids with no review row fall back rather than borrowing a mark.
    expect(resolveBrandIdentity('mistral-api')).toBeNull();
    expect(resolveBrandIdentity('muse-code')).toBeNull();

    expect(brandIconProps('definitely-not-a-brand').assetSrc).toBeNull();
    expect(brandIconProps(null).fallbackGlyph).toBe('box');
  });
});

describe('reviewed asset hygiene', () => {
  it('keeps every bundled asset static, offline and raster-free', () => {
    for (const [identity, record] of bundled) {
      const asset = record.asset;
      if (asset === null) throw new Error(`${identity} is not bundled`);
      const text = stripW3cNamespaceDeclarations(
        readFileSync(join(brandsDir, asset.file)).toString('utf8')
      );

      for (const forbidden of [
        '<script',
        'http://',
        'https://',
        '@import',
        '<image',
        '<foreignObject',
        'data:',
        'xlink:href',
        'base64',
      ]) {
        expect(text, `${identity} asset must not contain ${forbidden}`).not.toContain(forbidden);
      }
    }
  });

  it('never fetches an icon at runtime', () => {
    for (const relative of ['lib/utils/brandIcons.ts', 'lib/components/BrandIcon.svelte']) {
      const source = readFileSync(join(srcRoot, relative), 'utf8');

      for (const forbidden of ['fetch(', 'XMLHttpRequest', 'http://', 'https://', 'import(']) {
        expect(source, `${relative} must not contain ${forbidden}`).not.toContain(forbidden);
      }
      expect(source, `${relative} must not build markup from a string`).not.toContain('{@html');
    }
  });
});
