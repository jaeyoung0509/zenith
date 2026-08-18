import { describe, it, expect } from 'vitest';
import { formatBytes, formatTimeAgo, formatDuration } from '../lib/utils/format';

describe('formatBytes', () => {
  it('formats 0 bytes correctly', () => {
    expect(formatBytes(0)).toBe('0 B');
  });

  it('formats KB, MB, GB, TB accurately', () => {
    expect(formatBytes(1024)).toBe('1 KB');
    expect(formatBytes(1.5 * 1024 * 1024)).toBe('1.5 MB');
    expect(formatBytes(18.4 * 1024 * 1024 * 1024)).toBe('18.4 GB');
    expect(formatBytes(2 * 1024 * 1024 * 1024 * 1024)).toBe('2 TB');
  });

  it('handles negative numbers', () => {
    expect(formatBytes(-1024)).toBe('-1 KB');
  });
});

describe('formatTimeAgo', () => {
  it('handles empty or recent timestamps', () => {
    expect(formatTimeAgo(undefined)).toBe('Never');
    const now = Math.floor(Date.now() / 1000);
    expect(formatTimeAgo(now - 10)).toBe('Just now');
    expect(formatTimeAgo(now - 300)).toBe('5 min ago');
    expect(formatTimeAgo(now - 7200)).toBe('2 hours ago');
    expect(formatTimeAgo(now - 86400 * 3)).toBe('3 days ago');
  });
});

describe('formatDuration', () => {
  it('formats seconds, minutes, and hours', () => {
    expect(formatDuration(45)).toBe('45s');
    expect(formatDuration(90)).toBe('1m 30s');
    expect(formatDuration(3660)).toBe('1h 1m');
  });
});
