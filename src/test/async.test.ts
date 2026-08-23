import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { withMinimumDuration } from '../lib/utils/async';

describe('withMinimumDuration', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('returns the wrapped result without extra delay when work is slow enough', async () => {
    const work = vi.fn(async () => {
      await vi.advanceTimersByTimeAsync(800);
      return 'result';
    });

    let resolved: string | undefined;
    const settled = withMinimumDuration(work(), 600).then((value) => {
      resolved = value;
    });

    await vi.advanceTimersByTimeAsync(800);
    await settled;

    expect(resolved).toBe('result');
  });

  it('extends short work to the minimum duration', async () => {
    const work = Promise.resolve('quick');

    let resolved: string | undefined;
    const settled = withMinimumDuration(work, 600).then((value) => {
      resolved = value;
    });

    await vi.advanceTimersByTimeAsync(599);
    expect(resolved).toBeUndefined();

    await vi.advanceTimersByTimeAsync(1);
    await settled;
    expect(resolved).toBe('quick');
  });

  it('propagates rejections from the wrapped promise', async () => {
    const failing = Promise.reject(new Error('boom'));

    await expect(withMinimumDuration(failing, 600)).rejects.toThrow('boom');
  });
});
