import { describe, expect, it } from 'vitest';
import type { ScanItem } from '../lib/models/types';
import { filterAndSortCleanupItems } from '../lib/utils/cleanup';

function item(overrides: Partial<ScanItem>): ScanItem {
  return {
    id: 'cache',
    signature_id: 'dev.cache',
    name: 'Cache',
    category: 'developer',
    risk: 'safe',
    path: '/tmp/cache',
    size: { logical: 1, allocated: 1 },
    file_count: 1,
    description: 'Generated cache',
    is_selected: false,
    exists: true,
    ...overrides,
  };
}

describe('filterAndSortCleanupItems', () => {
  it('hides nonexistent and empty signature locations', () => {
    const result = filterAndSortCleanupItems(
      [
        item({ id: 'real' }),
        item({ id: 'missing', exists: false }),
        item({ id: 'empty', size: { logical: 0, allocated: 0 } }),
      ],
      'all',
      '',
      'size'
    );
    expect(result.map((entry) => entry.id)).toEqual(['real']);
  });

  it('orders by reclaimable bytes and keeps rebuild items opt-in', () => {
    const result = filterAndSortCleanupItems(
      [
        item({ id: 'small', size: { logical: 10, allocated: 10 } }),
        item({ id: 'large', risk: 'rebuild', size: { logical: 100, allocated: 100 } }),
      ],
      'all',
      '',
      'size'
    );
    expect(result.map((entry) => entry.id)).toEqual(['large', 'small']);
    expect(result[0].is_selected).toBe(false);
  });

  it('filters by risk and searchable metadata', () => {
    const result = filterAndSortCleanupItems(
      [
        item({ id: 'cargo', name: 'Cargo Registry', risk: 'rebuild', path: '/cargo/cache' }),
        item({ id: 'npm', name: 'npm Cache', path: '/npm/cache' }),
        item({ id: 'ollama', name: 'Ollama Blobs', risk: 'manual', path: '/ollama/models' }),
      ],
      'manual',
      'ollama',
      'name'
    );
    expect(result.map((entry) => entry.id)).toEqual(['ollama']);
  });
});
