/**
 * Brand identity registry for the White & Mint redesign (issue #280, section 7).
 *
 * Every product Zenith can display resolves either to a reviewed local asset
 * under `src/lib/assets/brands/` or to a neutral Lucide fallback glyph. The
 * review record for each bundled file — official source URL, upstream revision,
 * quoted license, required notices — is `docs/design/brand-assets.md`; `sha256`
 * here is the reviewed byte identity of the local copy, so the focused test
 * fails when a bundled file drifts from what was reviewed.
 *
 * Rules this table encodes:
 * - Assets are bundled files. No icon CDN, no favicon taken from an arbitrary
 *   domain, no dynamic import of a remote URL, no network call at runtime.
 * - A mark is bundled only when the copyright holder's own license covers
 *   redistributing that file (for example a logo committed to an MIT or
 *   Apache-2.0 repository the holder owns). Trademark permissions, press kits
 *   and "request permission" pages are not redistribution permission; those
 *   identities carry a fallback glyph and an `unresolvedReason` instead.
 * - Assets are never restyled: no CSS filter, recolor, distortion or clipping.
 *   Where a holder publishes several variants, the untouched file for Zenith's
 *   light-first surfaces is bundled.
 *
 * Identities, not aliases, are the keys: an alias only has to reach the row the
 * app already names. `copilot` and `gemini` also appear as integration
 * `tool_id`s, and `grok-build` is the `ProviderId` spelling of Zenith's Grok
 * integration, so both spellings resolve to one review row.
 */
import {
  Asterisk,
  Bot,
  Box,
  Code,
  Container,
  Cpu,
  FaceSlightlySmiling,
  Ghost,
  Monitor,
  Mountain,
  MousePointer2,
  Orbit,
  Package,
  Rocket,
  Route,
  Sparkles,
  Star,
  Terminal,
  Zap,
  type LucideIcon,
} from '@lucide/svelte';

/**
 * `?url&no-inline` keeps every reviewed file a separate emitted asset: a
 * reviewed mark is never base64-inlined, so what ships stays byte-identical to
 * the `sha256` recorded for review.
 */
import copilotAsset from '../assets/brands/copilot.png?url&no-inline';
import geminiAsset from '../assets/brands/gemini.png?url&no-inline';
import ollamaAsset from '../assets/brands/ollama.svg?url&no-inline';
import opencodeAsset from '../assets/brands/opencode.svg?url&no-inline';
import viteAsset from '../assets/brands/vite.svg?url&no-inline';
import vscodeAsset from '../assets/brands/vscode.png?url&no-inline';
import zenithAsset from '../assets/brands/zenith.svg?url&no-inline';

/**
 * Canonical identity ids: the AI provider ids Zenith already uses
 * (`src/lib/bindings/tauri.ts`) plus the tool identities this app can inspect.
 * Provider ids outside this list (`muse-code`, `meta-model-api`, `mistral-api`,
 * `fireworks-api`) are intentionally not identities here; they resolve to
 * `null` and render the neutral default glyph.
 */
export type BrandIdentity =
  | 'antigravity'
  | 'claude'
  | 'codex'
  | 'copilot'
  | 'cursor'
  | 'gemini'
  | 'grok'
  | 'opencode'
  | 'openrouter'
  | 'docker'
  | 'huggingface'
  | 'lmstudio'
  | 'mlx'
  | 'npm'
  | 'ollama'
  | 'vite'
  | 'vscode'
  | 'zenith';

/** Names of the neutral glyphs a resolved-but-unbundled identity renders. */
export type BrandFallbackGlyph =
  | 'asterisk'
  | 'bot'
  | 'box'
  | 'code'
  | 'container'
  | 'cpu'
  | 'face-slightly-smiling'
  | 'ghost'
  | 'monitor'
  | 'mountain'
  | 'mouse-pointer'
  | 'orbit'
  | 'package'
  | 'rocket'
  | 'route'
  | 'sparkles'
  | 'star'
  | 'terminal'
  | 'zap';

/** Sizes a caller may ask for; a brand minimum may raise the rendered size. */
export type BrandIconSize = 20 | 24 | 32;

export interface BrandAsset {
  /** Reviewed file name inside `src/lib/assets/brands/`. */
  readonly file: string;
  /** Vite-resolved URL used as the `<img>` source. */
  readonly src: string;
  /** sha256 of the reviewed bytes. */
  readonly sha256: string;
}

export interface BrandIdentityRecord {
  /** Product name; the adjacent text that owns the accessible name. */
  readonly label: string;
  /** Alternative spellings and integration ids that resolve to this row. */
  readonly aliases: readonly string[];
  /** Smallest size at which Zenith's reviewed artwork stays legible. */
  readonly minSizePx: BrandIconSize;
  /** Neutral Lucide glyph rendered when no asset is bundled. */
  readonly clearSpacePx?: number;
  readonly fallbackGlyph: BrandFallbackGlyph;
  /** Bundled reviewed asset, or `null` when the identity is unresolved. */
  readonly asset: BrandAsset | null;
  /** Why nothing is bundled; present exactly when `asset` is `null`. */
  readonly unresolvedReason: string | null;
}

/** Lucide glyphs available to the registry, keyed by registry glyph name. */
export const brandFallbackGlyphs: Record<BrandFallbackGlyph, LucideIcon> = {
  asterisk: Asterisk,
  bot: Bot,
  box: Box,
  code: Code,
  container: Container,
  cpu: Cpu,
  'face-slightly-smiling': FaceSlightlySmiling,
  ghost: Ghost,
  monitor: Monitor,
  mountain: Mountain,
  'mouse-pointer': MousePointer2,
  orbit: Orbit,
  package: Package,
  rocket: Rocket,
  route: Route,
  sparkles: Sparkles,
  star: Star,
  terminal: Terminal,
  zap: Zap,
};

/** Artwork floor used when an identity declares no minimum of its own. */
export const DEFAULT_MIN_SIZE_PX: BrandIconSize = 24;

/** Glyph for an unknown or absent identity. */
export const DEFAULT_FALLBACK_GLYPH: BrandFallbackGlyph = 'box';

export const BRAND_IDENTITIES: Record<BrandIdentity, BrandIdentityRecord> = {
  antigravity: {
    label: 'Antigravity',
    aliases: ['google-antigravity', 'antigravity-ide'],
    minSizePx: 20,
    fallbackGlyph: 'rocket',
    asset: null,
    unresolvedReason:
      'Google publishes no Antigravity repository or redistributable asset (github.com/google-antigravity/antigravity does not exist); the mark is published only on Google sites.',
  },
  claude: {
    label: 'Claude',
    aliases: ['anthropic', 'anthropic-api', 'claude-code', 'claude-cli'],
    minSizePx: 20,
    fallbackGlyph: 'asterisk',
    asset: null,
    unresolvedReason:
      'Anthropic\'s permissively licensed repositories commit no Claude mark; the only image in anthropics/anthropic-sdk-python (MIT) is an 800x90 ANTHROPIC wordmark, which is not an icon-slot mark.',
  },
  codex: {
    label: 'OpenAI Codex',
    aliases: ['openai', 'openai-api', 'codex-cli', 'openai-codex'],
    minSizePx: 20,
    fallbackGlyph: 'sparkles',
    asset: null,
    unresolvedReason:
      'openai/codex (Apache-2.0) commits no OpenAI or Codex mark — only 14 px skill-sample glyphs and a CI splash screenshot — and OpenAI publishes its marks under brand guidelines that grant no redistribution right.',
  },
  copilot: {
    label: 'GitHub Copilot',
    aliases: ['github-copilot', 'copilot-cli'],
    minSizePx: 24,
    fallbackGlyph: 'ghost',
    asset: {
      file: 'copilot.png',
      src: copilotAsset,
      sha256: 'cb853b5b8a881d697cd6e51439ebbc7ac1eea436ce450371fb948093ea509217',
    },
    unresolvedReason: null,
  },
  cursor: {
    label: 'Cursor',
    aliases: ['cursor-ide', 'cursor-agent', 'cursor-cli'],
    minSizePx: 20,
    fallbackGlyph: 'mouse-pointer',
    asset: null,
    unresolvedReason:
      'Cursor publishes no source or brand-asset repository (github.com/cursor/cursor commits no artwork and no LICENSE); its mark is published only on cursor.com.',
  },
  docker: {
    label: 'Docker',
    aliases: ['docker-desktop'],
    minSizePx: 24,
    fallbackGlyph: 'container',
    asset: null,
    unresolvedReason:
      'Docker grants no redistribution right for its whale mark (newsroom and brand pages only); the permissively licensed whale in moby/moby is the Moby project\'s logo, a different project\'s mark.',
  },
  gemini: {
    label: 'Gemini',
    aliases: ['google-gemini', 'gemini-cli'],
    minSizePx: 24,
    fallbackGlyph: 'star',
    asset: {
      file: 'gemini.png',
      src: geminiAsset,
      sha256: '351e9f5b1bf863d738cd7be4ed040a625a1419450ae7fc490143e4042b7c2438',
    },
    unresolvedReason: null,
  },
  grok: {
    label: 'Grok',
    aliases: ['grok-build', 'grok-cli', 'xai', 'xai-api'],
    minSizePx: 20,
    fallbackGlyph: 'orbit',
    asset: null,
    unresolvedReason:
      'xai-org/grok-1 and xai-org/xai-sdk-python (Apache-2.0) commit no xAI or Grok mark; the grok-1 tree contains no image asset at all.',
  },
  huggingface: {
    label: 'Hugging Face',
    aliases: ['hugging-face', 'hf', 'huggingface-hub'],
    minSizePx: 20,
    fallbackGlyph: 'face-slightly-smiling',
    asset: null,
    unresolvedReason:
      'Hugging Face\'s brand-asset repository is access-controlled (huggingface.co/huggingface/brand-assets answers HTTP 401 unauthenticated) and its Apache-2.0 repositories commit no company mark — chat-ui\'s committed logo is a HuggingChat product asset.',
  },
  lmstudio: {
    label: 'LM Studio',
    aliases: ['lm-studio'],
    minSizePx: 20,
    fallbackGlyph: 'monitor',
    asset: null,
    unresolvedReason:
      'LM Studio is proprietary and lmstudio-ai/lms (MIT) commits no image asset; the mark is published only on lmstudio.ai.',
  },
  mlx: {
    label: 'Apple MLX',
    aliases: ['apple-mlx', 'ml-explore'],
    minSizePx: 20,
    fallbackGlyph: 'cpu',
    asset: null,
    unresolvedReason:
      'ml-explore/mlx (MIT) commits only the 433x139 MLX wordmark (docs/logo/mlx_logo.svg); no compact MLX mark is published under a redistribution grant.',
  },
  npm: {
    label: 'npm',
    aliases: ['npm-cli'],
    minSizePx: 20,
    fallbackGlyph: 'package',
    asset: null,
    unresolvedReason:
      'npm/logos ("official logos for npm, Inc") publishes no LICENSE file, and npm/cli (Artistic-2.0) commits only its Arborist workspace logo; no redistribution grant covers an npm mark.',
  },
  ollama: {
    label: 'Ollama',
    aliases: [],
    minSizePx: 24,
    fallbackGlyph: 'bot',
    asset: {
      file: 'ollama.svg',
      src: ollamaAsset,
      sha256: '1c8cec1ca568fad4ca8e66ab0328ca7c44a4a87a23b644cf9bd0cbd024d040f1',
    },
    unresolvedReason: null,
  },
  opencode: {
    label: 'OpenCode',
    aliases: ['open-code', 'opencode-cli'],
    minSizePx: 24,
    fallbackGlyph: 'terminal',
    asset: {
      file: 'opencode.svg',
      src: opencodeAsset,
      sha256: 'b1c48d82ebd304eab820658387642bcf4c9b8350d6860894d0fe21ddca25ef3f',
    },
    unresolvedReason: null,
  },
  openrouter: {
    label: 'OpenRouter',
    aliases: ['open-router', 'openrouter-ai'],
    minSizePx: 20,
    fallbackGlyph: 'route',
    asset: null,
    unresolvedReason:
      'OpenRouterTeam/docs commits logo wordmarks but no LICENSE file (HTTP 404 for /LICENSE), so no redistribution grant was found; the archived MIT runner repository ships no logo.',
  },
  vite: {
    label: 'Vite',
    aliases: ['vitejs'],
    minSizePx: 20,
    fallbackGlyph: 'zap',
    asset: {
      file: 'vite.svg',
      src: viteAsset,
      sha256: 'ceeac38434be7a3b4d0f68b8cd8aa2b9ae78c260d6343087c6e095f8031ce4ff',
    },
    unresolvedReason: null,
  },
  vscode: {
    label: 'Visual Studio Code',
    aliases: ['vs-code', 'visual-studio-code', 'vscode-ide'],
    minSizePx: 24,
    fallbackGlyph: 'code',
    asset: {
      file: 'vscode.png',
      src: vscodeAsset,
      sha256: 'a51653af6534e0637d91b55ebd5f63f989302071d67a0724e48adfc16624b0a7',
    },
    unresolvedReason: null,
  },
  zenith: {
    label: 'Zenith',
    aliases: ['zenith-app'],
    minSizePx: 20,
    fallbackGlyph: 'mountain',
    asset: {
      file: 'zenith.svg',
      src: zenithAsset,
      sha256: '48d63018f5baa8772d840d77d2a64596ea126a16cfb1ef6bea0b2357ac241df5',
    },
    unresolvedReason: null,
  },
};

/** Case- and separator-insensitive lookup key for ids and aliases. */
export function normalizeBrandKey(raw: string): string {
  return raw.trim().toLowerCase().replace(/[\s_]+/g, '-');
}

const identityByKey: Map<string, BrandIdentity> = new Map();
for (const [identity, record] of Object.entries(BRAND_IDENTITIES) as Array<
  [BrandIdentity, BrandIdentityRecord]
>) {
  for (const candidate of [identity, ...record.aliases]) {
    const key = normalizeBrandKey(candidate);
    if (!identityByKey.has(key)) identityByKey.set(key, identity);
  }
}

/**
 * Maps a provider id, integration id or display name onto a registry identity.
 * Case-insensitive, whitespace- and underscore-tolerant, `null` when unknown.
 */
export function resolveBrandIdentity(raw: string | null | undefined): BrandIdentity | null {
  if (typeof raw !== 'string') return null;
  const key = normalizeBrandKey(raw);
  return key.length === 0 ? null : identityByKey.get(key) ?? null;
}

export interface BrandIconProps {
  /** Bundled asset URL, or `null` when the identity is unresolved. */
  readonly assetSrc: string | null;
  /** Render floor in px, before a caller-supplied size raises it. */
  readonly minSizePx: BrandIconSize;
  /** Glyph to render when `assetSrc` is `null`. */
  readonly fallbackGlyph: BrandFallbackGlyph;
}

/** Derives the icon an identity, alias or unknown string should render. */
export function brandIconProps(identity: string | null | undefined): BrandIconProps {
  const resolved = resolveBrandIdentity(identity);
  const record = resolved === null ? null : BRAND_IDENTITIES[resolved];
  return {
    assetSrc: record?.asset?.src ?? null,
    minSizePx: record?.minSizePx ?? DEFAULT_MIN_SIZE_PX,
    fallbackGlyph: record?.fallbackGlyph ?? DEFAULT_FALLBACK_GLYPH,
  };
}
