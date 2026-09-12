import goldenCapabilities from '../bindings/platform-capabilities.golden.json';
import type { PlatformCapabilities, PlatformKind } from './types';

/**
 * The backend generates `platform-capabilities.golden.json` from the same
 * `PlatformCapabilities` values it serves over IPC. Reading it here keeps the
 * browser preview and the frontend tests from inventing a platform matrix the
 * Rust build would never produce.
 */
export const goldenCapabilitiesByPlatform = goldenCapabilities as unknown as Record<
  PlatformKind,
  PlatformCapabilities
>;
