/** @vitest-environment jsdom */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { FrontendErrorStore, frontendErrorStore } from '../lib/stores/frontendErrors.svelte';

describe('FrontendErrorStore', () => {
  afterEach(() => {
    frontendErrorStore.clear();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('records safe metadata for uncaught errors and unhandled rejections', async () => {
    const remove = frontendErrorStore.captureFrom(window);

    window.dispatchEvent(
      Object.assign(new Event('error'), { message: 'token=secret123 /Users/apple/private' }) as ErrorEvent
    );
    const rejection = new Event('unhandledrejection') as PromiseRejectionEvent & {
      reason?: unknown;
    };
    rejection.reason = new Error('session secret123');
    window.dispatchEvent(rejection);

    expect(frontendErrorStore.entries.map((entry) => entry.kind)).toEqual([
      'error',
      'rejection',
    ]);
    expect(frontendErrorStore.entries[0].message).toBe('The interface encountered an unexpected error.');
    expect(frontendErrorStore.entries[1].message).toBe('A background operation failed unexpectedly.');
    expect(JSON.stringify(frontendErrorStore.entries)).not.toMatch(/secret123|\/Users\/apple/);
    expect(frontendErrorStore.entries[0].at).toMatch(/^\d{4}-\d{2}-\d{2}T/);

    remove();
    await Promise.resolve();
    window.dispatchEvent(Object.assign(new Event('error'), { message: 'ignored' }) as ErrorEvent);
    expect(frontendErrorStore.entries).toHaveLength(2);
  });

  it('keeps the newest entries and drops the oldest past the cap', () => {
    const store = new FrontendErrorStore();
    for (let index = 0; index < 25; index += 1) store.push('error');

    expect(store.entries).toHaveLength(20);
    expect(store.entries[0].id).toBe(5);
    expect(store.entries[19].id).toBe(24);

    store.clear();
    expect(store.entries).toEqual([]);
  });

  it('does not inspect or serialize arbitrary rejection reasons', () => {
    const store = new FrontendErrorStore();
    const remove = store.captureFrom(window);

    const rejection = new Event('unhandledrejection') as PromiseRejectionEvent & {
      reason?: unknown;
    };
    const inspect = vi.fn(() => { throw new Error('must not be called'); });
    rejection.reason = { toJSON: inspect, toString: inspect };
    window.dispatchEvent(rejection);

    expect(inspect).not.toHaveBeenCalled();
    expect(store.entries[0].message).toBe('A background operation failed unexpectedly.');
    remove();
  });

  it('assigns a stable unique key when duplicate errors share a timestamp', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-09-12T12:00:00.000Z'));
    const store = new FrontendErrorStore();

    store.push('error');
    store.push('error');

    expect(store.entries[0].at).toBe(store.entries[1].at);
    expect(store.entries[0].id).not.toBe(store.entries[1].id);
  });
});
