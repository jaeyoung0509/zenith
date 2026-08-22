import { describe, expect, it, vi } from 'vitest';
import { render } from 'svelte/server';
import StorageTools from '../routes/dashboard/StorageTools.svelte';

describe('StorageTools navigation cards', () => {
  it('makes the full Large Files and Applications cards keyboard-accessible buttons', () => {
    const rendered = render(StorageTools, {
      props: {
        onOpenLargeFiles: vi.fn(),
        onOpenApplications: vi.fn(),
      },
    });

    expect(rendered.body.match(/<button/g)).toHaveLength(2);
    expect(rendered.body).toContain('aria-label="Open Large Files"');
    expect(rendered.body).toContain('aria-label="Open Applications"');
    expect(rendered.body).toContain('w-full rounded-xl border');
    expect(rendered.body).toContain('focus-visible:ring-1');
  });
});
