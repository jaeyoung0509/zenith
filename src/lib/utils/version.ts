/**
 * Zenith Application Version Utility
 */

export const APP_VERSION: string = typeof __APP_VERSION__ !== 'undefined' ? __APP_VERSION__ : '0.1.0';

export function formatVersion(version: string = APP_VERSION): string {
  return version.startsWith('v') ? version : `v${version}`;
}
