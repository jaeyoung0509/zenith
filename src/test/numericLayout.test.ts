import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import ByteValue from '../lib/components/ByteValue.svelte';
import MemoryView from '../routes/dashboard/MemoryView.svelte';
import { memoryStore } from '../lib/stores/memory.svelte';

afterEach(() => { memoryStore.memory = null; });

describe('numeric layout contract', () => {
  it.each([0, 9.9 * 1024 ** 2, 10 * 1024 ** 3, 100 * 1024 ** 3, 1023.9 * 1024 ** 3])('keeps the complete byte value %s inside one no-wrap tabular element', (bytes) => {
    const { body } = render(ByteValue, { props: { bytes } });
    expect(body).toContain('data-byte-value');
    expect(body).toContain('whitespace-nowrap');
    expect(body).toContain('tabular-nums');
  });

  it('reserves the same metric and action columns for protected and terminable processes', () => {
    memoryStore.memory = {
      timestamp: 1000,
      total_bytes: 16 * 1024 ** 3, used_bytes: 10 * 1024 ** 3, available_bytes: 3 * 1024 ** 3,
      free_bytes: 610 * 1024 ** 2, compressed_bytes: 2.4 * 1024 ** 3, swap_used_bytes: 0,
      swap_total_bytes: 0, pressure: 'normal', top_processes: [
        { pid: 1, pids: [1], name: '한국어 개발 앱', memory_bytes: 100 * 1024 ** 3, process_count: 100, can_terminate: true },
        { pid: 2, pids: [2], name: 'Protected', memory_bytes: 9 * 1024 ** 2, process_count: 1, can_terminate: false },
      ],
    };
    const { body } = render(MemoryView);
    expect(body.match(/w-\[10ch\]/g)).toHaveLength(2);
    expect(body.match(/w-16 shrink-0/g)).toHaveLength(2);
    expect(body).toContain('한국어 개발 앱');
    expect(body).toContain('min-h-16');
  });
});
