import { describe, it, expect } from 'vitest';
import { formatCountdown, ttlRemaining } from '../lib/utils/format';

describe('formatCountdown', () => {
  it('returns expired for 0 and negative', () => {
    expect(formatCountdown(0)).toBe('expired');
    expect(formatCountdown(-10)).toBe('expired');
  });

  it('formats seconds below 60', () => {
    expect(formatCountdown(1)).toBe('1s remaining');
    expect(formatCountdown(59)).toBe('59s remaining');
  });

  it('formats minutes and seconds', () => {
    expect(formatCountdown(60)).toBe('1:00 remaining');
    expect(formatCountdown(61)).toBe('1:01 remaining');
    expect(formatCountdown(125)).toBe('2:05 remaining');
    expect(formatCountdown(3599)).toBe('59:59 remaining');
  });
});

describe('ttlRemaining', () => {
  it('computes remaining seconds from expiresAt and now', () => {
    const now = Date.now();
    const expiresAt = Math.floor(now / 1000) + 300; // 5m
    expect(ttlRemaining(expiresAt, now)).toBe(300);
    expect(ttlRemaining(expiresAt, now + 60 * 1000)).toBe(240);
    expect(ttlRemaining(expiresAt, now + 300 * 1000)).toBe(0);
    expect(ttlRemaining(expiresAt, now + 400 * 1000)).toBe(0);
  });

  it('handles boundary at TTL 300 and 900', () => {
    const now = 1_000_000_000_000;
    const nowSecs = Math.floor(now / 1000);
    // Plan TTL 300s boundary
    expect(ttlRemaining(nowSecs + 299, now)).toBe(299);
    expect(ttlRemaining(nowSecs + 300, now)).toBe(300);
    expect(ttlRemaining(nowSecs + 300, now + 300 * 1000)).toBe(0);
    // Inventory TTL 900s
    expect(ttlRemaining(nowSecs + 899, now)).toBe(899);
    expect(ttlRemaining(nowSecs + 900, now + 900 * 1000)).toBe(0);
  });
});
