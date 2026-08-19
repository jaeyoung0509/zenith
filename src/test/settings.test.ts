import { describe, expect, it } from 'vitest';
import type { DashboardTab } from '../lib/models/types';
import { moveOrdered, reorderOrdered, toggleOrdered } from '../lib/utils/quickPanel';

describe('dashboard tab reordering and customization', () => {
  const defaultTabs: DashboardTab[] = ['storage', 'docker', 'models', 'memory', 'usage', 'awake'];

  it('moves dashboard tabs up and down correctly', () => {
    const movedUp = moveOrdered(defaultTabs, 'docker', -1);
    expect(movedUp[0]).toBe('docker');
    expect(movedUp[1]).toBe('storage');

    const movedDown = moveOrdered(defaultTabs, 'storage', 1);
    expect(movedDown[0]).toBe('docker');
    expect(movedDown[1]).toBe('storage');
  });

  it('reorders dashboard tabs via drag and drop', () => {
    const reordered = reorderOrdered(defaultTabs, 'memory', 'storage');
    expect(reordered).toEqual(['memory', 'storage', 'docker', 'models', 'usage', 'awake']);
  });

  it('clamps tab movement at array boundaries', () => {
    const topStay = moveOrdered(defaultTabs, 'storage', -1);
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
    const singleTab: DashboardTab[] = ['storage'];
    const preserved = toggleOrdered(singleTab, 'storage', true);
    expect(preserved).toEqual(['storage']);
  });
});

describe('quick panel sections customization', () => {
  const defaultSections = ['cleanup', 'storage', 'memory', 'ai_usage'] as const;

  it('reorders quick panel sections via drag and drop', () => {
    const reordered = reorderOrdered([...defaultSections], 'memory', 'cleanup');
    expect(reordered).toEqual(['memory', 'cleanup', 'storage', 'ai_usage']);
  });

  it('moves quick panel sections with boundary clamping', () => {
    const moved = moveOrdered([...defaultSections], 'storage', -1);
    expect(moved).toEqual(['storage', 'cleanup', 'memory', 'ai_usage']);
  });
});
