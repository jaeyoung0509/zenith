import goldenContexts from '../bindings/platform-context.golden.json';
import type { PlatformContext, PlatformKind } from './types';

/** Platforms the browser preview can describe. */
export type PreviewPlatform = Extract<PlatformKind, 'macos' | 'windows' | 'linux'>;

/**
 * The platform vocabulary the backend serves for each platform.
 *
 * `export_platform_context_golden` generates this file from
 * `PlatformContext::for_platform`, so the browser preview shows the backend's
 * own nouns for the selected platform instead of a hand-written second copy.
 */
export const goldenPlatformContextByPlatform = goldenContexts as unknown as Record<
  PreviewPlatform,
  PlatformContext
>;
