import { describe, expect, it } from 'vitest';
import { APP_VERSION, formatVersion } from '../lib/utils/version';

describe('version utilities', () => {
  it('defines a valid semver APP_VERSION', () => {
    expect(APP_VERSION).toBeDefined();
    expect(typeof APP_VERSION).toBe('string');
    expect(APP_VERSION).toMatch(/^\d+\.\d+\.\d+/);
  });

  it('formats versions with v prefix', () => {
    expect(formatVersion('0.1.0')).toBe('v0.1.0');
    expect(formatVersion('v0.1.0')).toBe('v0.1.0');
    expect(formatVersion()).toBe(`v${APP_VERSION}`);
  });
});
