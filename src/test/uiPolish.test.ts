import { readFileSync } from 'node:fs';
import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import type { CleanFailureReason, CleanResult } from '../lib/models/types';
import CleanResultModal from '../lib/components/CleanResultModal.svelte';
import QuickPanel from '../routes/quick/QuickPanel.svelte';
import SettingsView from '../routes/dashboard/SettingsView.svelte';
import ModelsView from '../routes/dashboard/ModelsView.svelte';
import Card from '../lib/components/Card.svelte';
import Button from '../lib/components/Button.svelte';
import { platformCapabilitiesStore } from '../lib/stores/platformCapabilities.svelte';
import { platformContextStore } from '../lib/stores/platformContext.svelte';
import { localModelsStore } from '../lib/stores/models.svelte';
import { goldenCapabilitiesByPlatform } from '../lib/models/platformCapabilities';
import { mockApi } from '../lib/api/mock';
import ReorderControls from '../lib/components/ReorderControls.svelte';

afterEach(() => {
  localModelsStore.models = [];
  platformCapabilitiesStore.reset();
  platformContextStore.reset();
});

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
  it('labels the icon-only model reveal action and gates it on platform capabilities', async () => {
    localModelsStore.models = [
      {
        id: 'model-reveal-contract',
        name: 'Llama 3 8B',
        source: 'ollama',
        path: '/Users/mock/.ollama/models/llama3',
        size_bytes: 4 * 1024 ** 3,
        format: 'gguf',
        parameter_size: '8B',
        quantization: 'Q4_K_M',
        last_modified: null,
      },
    ];
    platformContextStore.context = await mockApi.getPlatformContext();
    platformCapabilitiesStore.capabilities = {
      ...goldenCapabilitiesByPlatform.macos,
      system_actions: { status: 'unavailable', reason: 'Revealing files is not supported here.' },
    };

    const unavailable = render(ModelsView);
    expect(unavailable.body).toContain('aria-label="Show Llama 3 8B in file manager"');
    expect(unavailable.body).toContain('title="Revealing files is not supported here."');
    expect(unavailable.body).toContain('disabled=""');

    platformCapabilitiesStore.capabilities = goldenCapabilitiesByPlatform.macos;
    const available = render(ModelsView);
    // The tooltip is the backend's own action label, not a hardcoded macOS noun.
    expect(available.body).toContain('title="Reveal in Finder"');
    expect(available.body).not.toContain('disabled=""');
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

  it('supports paint-only feedback for controls that must avoid compositor promotion', () => {
    const rendered = render(Button, { props: { motion: 'paint' } });

    expect(rendered.body).toContain(
      'transition-[background-color,color,border-color]'
    );
    expect(rendered.body).not.toContain(
      'transition-[background-color,color,border-color,transform,opacity]'
    );
    expect(rendered.body).not.toContain('active:scale-[0.98]');
  });
});

describe('cleanup result feedback', () => {
  const result = (status: 'success' | 'partial' | 'failed', success: boolean, error_message: string | null = null): CleanResult => ({
    plan_id: 'plan-1',
    started_at: 1,
    finished_at: 2,
    total_reclaimed_bytes: success ? 1024 : 0,
    total_failed_bytes: success ? 0 : 1024,
    // The backend counts what it could not fully clean; a `partial` target
    // reclaimed bytes, a `failed` one reclaimed nothing.
    partial_count: status === 'partial' ? 1 : 0,
    failed_count: status === 'failed' ? 1 : 0,
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

    const failedResult = result('failed', false, 'Permission denied (os error 13)');
    failedResult.actual_disk_free_delta = 4096;
    const failed = render(CleanResultModal, {
      props: { result: failedResult, onClose: () => undefined },
    });
    expect(failed.body).toContain('Clean Failed');
    expect(failed.body).toContain('No storage was reclaimed');
    expect(failed.body).not.toContain('Clean Complete');
    expect(failed.body).not.toContain('Free space delta');

    expect(success.body).not.toContain('target(s) partially cleaned');
    expect(partial.body).toContain('1 target(s) partially cleaned, 0 failed');
    expect(failed.body).toContain('0 target(s) partially cleaned, 1 failed');
  });

  it('reports a run that both partially cleaned and failed targets', () => {
    const mixed: CleanResult = {
      ...result('partial', true, '2 file(s) could not be removed: busy'),
      partial_count: 1,
      failed_count: 1,
      items: [
        {
          item_id: 'item-1',
          name: 'Partial cache',
          path: '/tmp/partial-cache',
          status: 'partial',
          success: true,
          bytes_reclaimed: 1024,
          failure_reason: null,
          error_message: '2 file(s) could not be removed: busy',
        },
        {
          item_id: 'item-2',
          name: 'Locked cache',
          path: '/tmp/locked-cache',
          status: 'failed',
          success: false,
          bytes_reclaimed: 0,
          failure_reason: 'in_use' as CleanFailureReason,
          error_message: 'Sharing violation (file in use by another process): os error 32',
        },
      ],
    };

    const rendered = render(CleanResultModal, {
      props: { result: mixed, onClose: () => undefined },
    });

    expect(rendered.body).toContain('Clean Partially Complete');
    expect(rendered.body).toContain('1 target(s) partially cleaned, 1 failed');
    expect(rendered.body).toContain('1 item(s) partially cleaned');
    expect(rendered.body).toContain('1 item(s) failed');
  });

  it('reflects intensive cleanup capability availability honestly in Settings', async () => {
    try {
      platformCapabilitiesStore.capabilities = {
        ...(await mockApi.getPlatformCapabilities()),
        platform: 'windows',
        intensive_cleanup: {
          status: 'unavailable',
          reason: 'Intensive cleanup is unavailable on Windows because no Windows-specific intensive signatures are defined.',
        },
      };

      const unavailable = render(SettingsView);
      expect(unavailable.body).toContain('Unavailable');
      expect(unavailable.body).toContain('Intensive cleanup is unavailable on Windows because no Windows-specific intensive signatures are defined.');

      platformCapabilitiesStore.capabilities = {
        ...(await mockApi.getPlatformCapabilities()),
        platform: 'macos',
        intensive_cleanup: { status: 'available' },
      };

      const available = render(SettingsView);
      expect(available.body).toContain('Opt-in');
    } finally {
      platformCapabilitiesStore.reset();
    }
  });
});
