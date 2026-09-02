import { describe, expect, it } from 'vitest';
import { mockApi } from '../lib/api/mock';
import { nativeApi } from '../lib/api/native';
import { mockStorageApi, nativeStorageApi } from '../lib/api/storage';

describe('browser preview API parity', () => {
  it('implements exactly the native top-level command surface', () => {
    expect(Object.keys(mockApi).sort()).toEqual(Object.keys(nativeApi).sort());
  });

  it('implements exactly the native storage workflow surface', () => {
    expect(Object.keys(mockStorageApi).sort()).toEqual(
      Object.keys(nativeStorageApi).sort()
    );
  });
});
