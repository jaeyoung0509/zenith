/** @vitest-environment jsdom */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { FrontendErrorStore, frontendErrorStore } from '../lib/stores/frontendErrors.svelte';

describe('FrontendErrorStore', () => {
  afterEach(() => {
    frontendErrorStore.clear();
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it('records an uncaught error and an unhandled rejection from the window', async () => {
    const remove = frontendErrorStore.captureFrom(window);

    window.dispatchEvent(
      Object.assign(new Event('error'), { message: 'renderer exploded' }) as ErrorEvent
    );
    const rejection = new Event('unhandledrejection') as PromiseRejectionEvent & {
      reason?: unknown;
    };
    rejection.reason = new Error('background refresh failed');
    window.dispatchEvent(rejection);

    expect(frontendErrorStore.entries.map((entry) => entry.kind)).toEqual([
      'error',
      'rejection',
    ]);
    expect(frontendErrorStore.entries[0].message).toBe('renderer exploded');
    expect(frontendErrorStore.entries[1].message).toBe('background refresh failed');
    expect(frontendErrorStore.entries[0].at).toMatch(/^\d{4}-\d{2}-\d{2}T/);

    remove();
    await Promise.resolve();
    window.dispatchEvent(Object.assign(new Event('error'), { message: 'ignored' }) as ErrorEvent);
    expect(frontendErrorStore.entries).toHaveLength(2);
  });

  it('keeps the newest entries and drops the oldest past the cap', () => {
    const store = new FrontendErrorStore();
    for (let index = 0; index < 25; index += 1) {
      store.push('error', `failure ${index}`);
    }

    expect(store.entries).toHaveLength(20);
    expect(store.entries[0].message).toBe('failure 5');
    expect(store.entries[19].message).toBe('failure 24');

    store.clear();
    expect(store.entries).toEqual([]);
  });

  it('describes a rejection that is not an Error without losing it', () => {
    const store = new FrontendErrorStore();
    const remove = store.captureFrom(window);

    const rejection = new Event('unhandledrejection') as PromiseRejectionEvent & {
      reason?: unknown;
    };
    rejection.reason = { code: 'E_BRIDGE' };
    window.dispatchEvent(rejection);

    expect(store.entries[0].message).toBe('{"code":"E_BRIDGE"}');
    remove();
  });

  it('replaces an empty message instead of rendering a blank row', () => {
    const store = new FrontendErrorStore();
    store.push('error', '   ');
    expect(store.entries[0].message).toBe('No message was provided');
  });

  it('assigns a stable unique key when duplicate errors share a timestamp', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-09-12T12:00:00.000Z'));
    const store = new FrontendErrorStore();

    store.push('error', 'duplicate');
    store.push('error', 'duplicate');

    expect(store.entries[0].at).toBe(store.entries[1].at);
    expect(store.entries[0].id).not.toBe(store.entries[1].id);
  });
});
