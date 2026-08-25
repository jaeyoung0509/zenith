import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import type { CleanFailureReason, CleanResult } from '../lib/models/types';
import CleanResultModal from '../lib/components/CleanResultModal.svelte';
import QuickPanel from '../routes/quick/QuickPanel.svelte';
import SettingsView from '../routes/dashboard/SettingsView.svelte';
import Card from '../lib/components/Card.svelte';
import Button from '../lib/components/Button.svelte';
import ReorderControls from '../lib/components/ReorderControls.svelte';

describe('theme surface contract', () => {
  it('lets the document body follow the active design tokens', () => {
    const html = readFileSync(new URL('../../index.html', import.meta.url), 'utf8');

    expect(html).toContain('bg-background text-foreground');
    expect(html).not.toContain('bg-[#121216]');
    expect(html).not.toContain('text-[#fafafa]');
  });
});

describe('compact-window layout contracts', () => {
  it('keeps the quick panel header and footer fixed around a shrinkable scroll region', () => {
    const rendered = render(QuickPanel);

    expect(rendered.body).toContain('min-h-0 flex-1 overflow-y-auto');
    expect(rendered.body).toContain('shrink-0 pt-3 border-t');
    expect(rendered.body).not.toContain('backdrop-blur-xl');
  });

  it('avoids unnecessary backdrop compositing on shared cards', () => {
    const rendered = render(Card);

    expect(rendered.body).not.toContain('backdrop-blur');
  });
});

describe('accessible action contracts', () => {
  it('labels icon-only model reveal actions', () => {
    const source = readFileSync(
      new URL('../routes/dashboard/ModelsView.svelte', import.meta.url),
      'utf8'
    );

    expect(source).toContain('ariaLabel={`Reveal ${model.name} in Finder`}');
    expect(source).toContain('title="Reveal in Finder"');
  });

  it('offers keyboard-operable ordering controls in Settings', () => {
    const rendered = render(SettingsView);

    expect(rendered.body).toContain('aria-label="Move Storage &amp; Disks up"');
    expect(rendered.body).toContain('aria-label="Move Storage &amp; Disks down"');
    expect(rendered.body).toContain('aria-label="Move Storage up"');
    expect(rendered.body).toContain('aria-label="Move Codex down"');
  });

  it('disables reorder actions at collection boundaries', () => {
    const first = render(ReorderControls, {
      props: { label: 'Storage', index: 0, count: 3, onMove: () => undefined },
    });
    const last = render(ReorderControls, {
      props: { label: 'Memory', index: 2, count: 3, onMove: () => undefined },
    });
    const buttonTag = (body: string, label: string) =>
      body.match(/<button[^>]*>/g)?.find((tag) => tag.includes(`aria-label="${label}"`)) ?? '';

    expect(buttonTag(first.body, 'Move Storage up')).toContain('disabled=""');
    expect(buttonTag(first.body, 'Move Storage down')).not.toContain('disabled=""');
    expect(buttonTag(last.body, 'Move Memory up')).not.toContain('disabled=""');
    expect(buttonTag(last.body, 'Move Memory down')).toContain('disabled=""');
  });

  it('limits button transitions to visual properties and keeps focus rings immediate', () => {
    const rendered = render(Button);

    expect(rendered.body).toContain(
      'transition-[background-color,color,border-color,transform,opacity]'
    );
    expect(rendered.body).not.toContain('transition-all');
  });
});

describe('cleanup result feedback', () => {
  const result = (status: 'success' | 'partial' | 'failed', success: boolean, error_message: string | null = null): CleanResult => ({
    plan_id: 'plan-1',
    started_at: 1,
    finished_at: 2,
    total_reclaimed_bytes: success ? 1024 : 0,
    total_failed_bytes: success ? 0 : 1024,
    items: [
      {
        item_id: 'item-1',
        name: 'Cache item',
        path: '/tmp/cache-item',
        status,
        success,
        bytes_reclaimed: success ? 1024 : 0,
        failure_reason: (success ? null : 'permission_denied') as CleanFailureReason | null,
        error_message,
      },
    ],
    actual_disk_free_delta: 0,
  });

  it('renders aggregate success, partial, and failure copy honestly', () => {
    const success = render(CleanResultModal, {
      props: { result: result('success', true), onClose: () => undefined },
    });
    expect(success.body).toContain('Clean Complete');
    expect(success.body).toContain('Storage has been safely reclaimed');

    const partial = render(CleanResultModal, {
      props: { result: result('partial', true, 'one file was locked'), onClose: () => undefined },
    });
    expect(partial.body).toContain('Clean Partially Complete');
    expect(partial.body).toContain('Some storage was reclaimed');

    const failed = render(CleanResultModal, {
      props: { result: result('failed', false, 'Permission denied (os error 13)'), onClose: () => undefined },
    });
    expect(failed.body).toContain('Clean Failed');
    expect(failed.body).toContain('No storage was reclaimed');
    expect(failed.body).not.toContain('Clean Complete');
    expect(failed.body).not.toContain('Free space delta: +0 B');
  });
});
