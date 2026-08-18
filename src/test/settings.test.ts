import { describe, expect, it } from 'vitest';
import type { DashboardTab } from '../lib/models/types';
import { moveOrdered, toggleOrdered } from '../lib/utils/quickPanel';

describe('dashboard tab reordering and customization', () => {
  const defaultTabs: DashboardTab[] = ['disk', 'storage', 'docker', 'models', 'memory', 'usage', 'awake'];

  it('moves dashboard tabs up and down correctly', () => {
    const movedUp = moveOrdered(defaultTabs, 'storage', -1);
    expect(movedUp[0]).toBe('storage');
    expect(movedUp[1]).toBe('disk');

    const movedDown = moveOrdered(defaultTabs, 'disk', 1);
    expect(movedDown[0]).toBe('storage');
    expect(movedDown[1]).toBe('disk');
  });

  it('clamps tab movement at array boundaries', () => {
    const topStay = moveOrdered(defaultTabs, 'disk', -1);
    expect(topStay).toEqual(defaultTabs);

    const bottomStay = moveOrdered(defaultTabs, 'awake', 1);
    expect(bottomStay).toEqual(defaultTabs);
  });

  it('toggles dashboard tabs on and off while preserving at least one tab', () => {
    const withoutDocker = toggleOrdered(defaultTabs, 'docker', true);
    expect(withoutDocker.includes('docker')).toBe(false);

    const withDockerBack = toggleOrdered(withoutDocker, 'docker', true);
    expect(withDockerBack[withDockerBack.length - 1]).toBe('docker');

    // Never remove last tab
    const singleTab: DashboardTab[] = ['disk'];
    const preserved = toggleOrdered(singleTab, 'disk', true);
    expect(preserved).toEqual(['disk']);
  });
});
