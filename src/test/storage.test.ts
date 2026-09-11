import { afterEach, describe, expect, it, vi, beforeEach } from 'vitest';
import { render } from 'svelte/server';
import { readFileSync } from 'node:fs';
import StorageView from '../routes/dashboard/StorageView.svelte';
import CategoryDetailView from '../routes/dashboard/CategoryDetailView.svelte';
import CleanResultModal from '../lib/components/CleanResultModal.svelte';
import { scanStore } from '../lib/stores/scan.svelte';
import { platformCapabilitiesStore } from '../lib/stores/platformCapabilities.svelte';
import { mockApi } from '../lib/api/mock';
import type { CategoryResult, CleanResult } from '../lib/models/types';

afterEach(() => {
  platformCapabilitiesStore.reset();
  scanStore.lastScan = null;
  scanStore.selectedMap = {};
  scanStore.isScanning = false;
  scanStore.isCleaning = false;
  scanStore.lastScanTrigger = null;
});

describe('StorageView CTA and responsive toolbar layout', () => {
  beforeEach(() => {
    scanStore.lastScan = null;
    scanStore.selectedMap = {};
    scanStore.isScanning = false;
    scanStore.isCleaning = false;
  });

  it('renders "Review cleanup" for safe-only selections without duplicating byte count in CTA text', () => {
    const mockCategory: CategoryResult = {
      category: 'ai',
      display_name: 'AI Tools',
      items: [
        {
          id: 'item-1',
          signature_id: 'sig.ai.1',
          name: 'Claude Cache',
          category: 'ai',
          risk: 'safe',
          path: '/tmp/claude',
          size: { logical: 1024 * 1024 * 100, allocated: 1024 * 1024 * 100 },
          file_count: 50,
          description: 'Claude session caches',
          is_selected: true,
          last_modified: Date.now(),
          exists: true,
        },
      ],
      total_bytes: 1024 * 1024 * 100,
      safe_bytes: 1024 * 1024 * 100,
      rebuild_bytes: 0,
      manual_bytes: 0,
    };

    const nowSeconds = Math.floor(Date.now() / 1000);
    scanStore.lastScan = {
      scan_id: 'scan-1',
      valid_for_seconds: 300,
      started_at: nowSeconds - 1,
      finished_at: nowSeconds,
      categories: [mockCategory],
      total_bytes: 1024 * 1024 * 100,
      safe_bytes: 1024 * 1024 * 100,
      rebuild_bytes: 0,
      manual_bytes: 0,
    };
    scanStore.selectedMap = { 'item-1': true };

    const rendered = render(StorageView, {
      props: {
        onSelectCategory: vi.fn(),
      },
    });

    expect(rendered.body).toContain('Review cleanup');
    expect(rendered.body).not.toContain('Clean Safely');
    // Ensure the CTA button strictly renders clean text without appended byte label
    expect(rendered.body).toContain('<span>Review cleanup</span>');
    expect(rendered.body).not.toContain('Review cleanup ·');
    expect(rendered.body).not.toContain('Review cleanup 100 MB');
    expect(rendered.body).not.toMatch(/Review cleanup\s*·?\s*\d+\s*(?:MB|GB|KB|B)/);
    // Ensure summary pill renders byte count separately
    expect(rendered.body).toMatch(/text-success[^>]*>✓ [\s\S]*?100 MB[\s\S]*? Safe<\/span>/);
    // Ensure responsive toolbar classes for 960x660 baseline
    expect(rendered.body).toContain('flex flex-col sm:flex-row sm:items-center justify-between gap-3');
    expect(rendered.body).toContain('aria-label="Cleanup selection and actions"');
    expect(rendered.body).toContain('aria-label="Open storage settings"');
  });

  it('renders "Review cleanup" when rebuildable items are selected', () => {
    const mockCategory: CategoryResult = {
      category: 'developer',
      display_name: 'Developer Caches',
      items: [
        {
          id: 'item-rebuild',
          signature_id: 'sig.dev.cargo',
          name: 'Cargo Target',
          category: 'developer',
          risk: 'rebuild',
          path: '/tmp/target',
          size: { logical: 1024 * 1024 * 500, allocated: 1024 * 1024 * 500 },
          file_count: 120,
          description: 'Rebuildable build artifacts',
          is_selected: true,
          last_modified: Date.now(),
          exists: true,
        },
      ],
      total_bytes: 1024 * 1024 * 500,
      safe_bytes: 0,
      rebuild_bytes: 1024 * 1024 * 500,
      manual_bytes: 0,
    };

    const nowSeconds = Math.floor(Date.now() / 1000);
    scanStore.lastScan = {
      scan_id: 'scan-2',
      valid_for_seconds: 300,
      started_at: nowSeconds - 1,
      finished_at: nowSeconds,
      categories: [mockCategory],
      total_bytes: 1024 * 1024 * 500,
      safe_bytes: 0,
      rebuild_bytes: 1024 * 1024 * 500,
      manual_bytes: 0,
    };
    scanStore.selectedMap = { 'item-rebuild': true };
    expect(scanStore.rebuildSelectedBytes).toBe(1024 * 1024 * 500);

    const rendered = render(StorageView, {
      props: {
        onSelectCategory: vi.fn(),
      },
    });

    expect(rendered.body).toContain('Review cleanup');
    // Ensure the CTA button strictly renders clean text without appended byte label
    expect(rendered.body).toContain('<span>Review cleanup</span>');
    expect(rendered.body).not.toContain('Review cleanup ·');
    expect(rendered.body).not.toContain('Review cleanup 500 MB');
    expect(rendered.body).not.toMatch(/Review cleanup\s*·?\s*\d+\s*(?:MB|GB|KB|B)/);
    // Ensure summary pill renders rebuildable count separately
    expect(rendered.body).toMatch(/text-warning[^>]*>↻ [\s\S]*?500 MB[\s\S]*? Rebuildable<\/span>/);
  });

  it('wires zero-byte manual selections to the shared toolbar contract', () => {
    const safeItem = {
      id: 'safe-item',
      signature_id: 'sig.safe',
      name: 'Safe Cache',
      category: 'developer' as const,
      risk: 'safe' as const,
      path: '/tmp/safe-cache',
      size: { logical: 1024, allocated: 1024 },
      file_count: 1,
      description: 'Safe cache',
      is_selected: true,
      last_modified: null,
      exists: true,
    };
    const manualItem = {
      ...safeItem,
      id: 'manual-item',
      signature_id: 'sig.manual',
      name: 'Manual Resource',
      path: '/tmp/manual-resource',
      risk: 'manual' as const,
      size: { logical: 0, allocated: 0 },
    };
    const category: CategoryResult = {
      category: 'developer',
      display_name: 'Developer Caches',
      items: [safeItem, manualItem],
      total_bytes: 1024,
      safe_bytes: 1024,
      rebuild_bytes: 0,
      manual_bytes: 0,
    };
    const nowSeconds = Math.floor(Date.now() / 1000);
    scanStore.lastScan = {
      scan_id: 'scan-manual-zero',
      valid_for_seconds: 300,
      started_at: nowSeconds - 1,
      finished_at: nowSeconds,
      categories: [category],
      total_bytes: 1024,
      safe_bytes: 1024,
      rebuild_bytes: 0,
      manual_bytes: 0,
    };
    scanStore.selectedMap = { 'safe-item': true, 'manual-item': true };

    const rendered = render(StorageView, { props: { onSelectCategory: vi.fn() } });

    expect(rendered.body).toContain('1 Manual item');
    expect(rendered.body).toContain('Manual items require their dedicated management action');
  });

  it('uses the header scan control as the only freshness status UI', () => {
    scanStore.isScanning = true;
    scanStore.lastScanTrigger = 'auto';

    const rendered = render(StorageView, { props: { onSelectCategory: vi.fn() } });

    expect(rendered.body).toContain('animate-loading-dash');
    expect(rendered.body).toContain('Scanning…');
    expect(rendered.body).not.toContain('Scan storage to find current cleanup candidates');
    expect(rendered.body).not.toContain('Scan Again');
  });

  it('renders the header scan control with a stable square wrapper and paint-only motion', () => {
    scanStore.isScanning = false;
    const idleRender = render(StorageView, { props: { onSelectCategory: vi.fn() } });
    expect(idleRender.body).toContain('id="storage-scan-button"');
    expect(idleRender.body).toContain('inline-flex items-center justify-center shrink-0 w-3.5 h-3.5');
    expect(idleRender.body).toContain('transition-[background-color,color,border-color]');

    scanStore.isScanning = true;
    const scanningRender = render(StorageView, { props: { onSelectCategory: vi.fn() } });
    expect(scanningRender.body).toContain('id="storage-scan-button"');
    expect(scanningRender.body).toContain('inline-flex items-center justify-center shrink-0 w-3.5 h-3.5');
    expect(scanningRender.body).toContain('animate-loading-dash');
  });

  it('suppresses native WebKit blue outline while preserving intentional focus-visible keyboard styling on tabpanel', () => {
    const rendered = render(StorageView, { props: { onSelectCategory: vi.fn() } });
    expect(rendered.body).toContain('role="tabpanel"');
    expect(rendered.body).toContain('tabindex="0"');
    expect(rendered.body).toContain('outline-none');
    expect(rendered.body).toContain('focus:outline-none');
    expect(rendered.body).toContain('focus-visible:ring-1');
  });

  it('renders CleanResultModal with accessible dialog semantics and deterministic done button', () => {
    const mockResult: CleanResult = {
      plan_id: 'plan-done',
      started_at: 1000,
      finished_at: 1005,
      total_reclaimed_bytes: 1024 * 1024 * 50,
      total_failed_bytes: 0,
      actual_disk_free_delta: 1024 * 1024 * 50,
      items: [
        {
          item_id: 'item-1',
          name: 'Xcode DerivedData',
          path: '/Users/test/Library/Developer/Xcode/DerivedData',
          bytes_reclaimed: 1024 * 1024 * 50,
          success: true,
          status: 'success',
          failure_reason: null,
          error_message: null,
        },
      ],
    };

    const rendered = render(CleanResultModal, {
      props: { result: mockResult, onClose: vi.fn() },
    });

    expect(rendered.body).toContain('<dialog');
    expect(rendered.body).toContain('aria-modal="true"');
    expect(rendered.body).toContain('aria-labelledby=');
    expect(rendered.body).toContain('aria-describedby=');
    expect(rendered.body).toContain('Clean Complete');
    expect(rendered.body).toContain('50 MB');
    expect(rendered.body).toContain('-done-button');
  });

  it('keeps Storage scan feedback off transform and opacity compositor animations', () => {
    const css = readFileSync(new URL('../app.css', import.meta.url), 'utf-8');
    const keyframesIndex = css.indexOf('@keyframes loading-dash');
    expect(keyframesIndex).toBeGreaterThan(0);
    const keyframesBlock = css.slice(keyframesIndex, css.indexOf('.animate-loading-dash'));
    expect(keyframesBlock).toContain('stroke-dashoffset:');
    expect(keyframesBlock).not.toContain('transform:');
    expect(keyframesBlock).not.toContain('rotate:');
    expect(keyframesBlock).not.toContain('opacity:');

    expect(css).toContain('@media (prefers-reduced-motion: reduce)');
    const reducedMotionIndex = css.indexOf('@media (prefers-reduced-motion: reduce)');
    const reducedMotionBlock = css.slice(reducedMotionIndex);
    expect(reducedMotionBlock).toContain('.animate-loading-dash');
    expect(reducedMotionBlock).toContain('animation: none !important');

    const storageView = readFileSync(
      new URL('../routes/dashboard/StorageView.svelte', import.meta.url),
      'utf-8'
    );
    const storageTools = readFileSync(
      new URL('../routes/dashboard/StorageTools.svelte', import.meta.url),
      'utf-8'
    );
    const quickPanel = readFileSync(
      new URL('../routes/quick/QuickPanel.svelte', import.meta.url),
      'utf-8'
    );

    expect(storageView).toContain('motion="paint"');
    expect(storageTools).toContain('motion="paint"');
    expect(storageView).toContain('<LoadingSpinner');
    expect(storageTools).toContain('<LoadingSpinner');
    expect(quickPanel).toContain('<LoadingSpinner');
    expect(storageView).not.toContain('animate-gentle-spin');
    expect(storageTools).not.toContain('animate-gentle-spin');
  });
});

describe('detected versus reclaimable storage copy', () => {
  it('labels manual container storage as detected and leaves it unselected', () => {
    const bytes = 6 * 1024 * 1024 * 1024;
    const category: CategoryResult = {
      category: 'container',
      display_name: 'Docker & Containers',
      items: [
        {
          id: 'container.orbstack.storage',
          signature_id: 'adapter.orbstack.storage',
          name: 'OrbStack VM Storage',
          category: 'container',
          risk: 'manual',
          path: '/Users/test/Library/Group Containers/HUAQ24HBR6.dev.orbstack/data/data.img.raw',
          size: { logical: 240 * 1024 * 1024 * 1024, allocated: bytes },
          file_count: 1,
          description: 'Active container and Linux VM data.',
          is_selected: false,
          last_modified: null,
          exists: true,
        },
      ],
      total_bytes: bytes,
      safe_bytes: 0,
      rebuild_bytes: 0,
      manual_bytes: bytes,
    };

    const nowSeconds = Math.floor(Date.now() / 1000);
    scanStore.lastScan = {
      scan_id: 'scan-orbstack',
      valid_for_seconds: 300,
      started_at: nowSeconds - 1,
      finished_at: nowSeconds,
      categories: [category],
      total_bytes: bytes,
      safe_bytes: 0,
      rebuild_bytes: 0,
      manual_bytes: bytes,
    };
    scanStore.syncSelectionFromScan(scanStore.lastScan);

    const rendered = render(CategoryDetailView, {
      props: { categoryResult: category, onBack: vi.fn(), onNavigateTab: vi.fn() },
    });

    expect(rendered.body).toContain('1 detected location');
    expect(rendered.body).toContain('6 GB detected');
    expect(rendered.body).not.toContain('reclaimable locations');
    expect(rendered.body).toContain('Zenith reports their storage without deleting it');
    expect(scanStore.selectedMap['container.orbstack.storage']).toBe(false);
    expect(scanStore.reclaimableBytes).toBe(0);
  });

  it('hides the Applications tab when installed_apps is unavailable', async () => {
    platformCapabilitiesStore.capabilities = {
      ...(await mockApi.getPlatformCapabilities()),
      platform: 'windows',
      installed_apps: {
        status: 'unavailable',
        reason: 'Windows application inventory is not supported. Use Windows Settings.',
      },
    };

    const rendered = render(StorageView, {
      props: { onSelectCategory: vi.fn() },
    });

    expect(rendered.body).not.toContain('Applications');
    expect(rendered.body).toContain('Cleanup');
    expect(rendered.body).toContain('Large Files');
    expect(rendered.body).toContain('Developer Artifacts');
    expect(rendered.body).toContain('Disks');
  });

  it('shows the Applications tab when installed_apps is available', async () => {
    platformCapabilitiesStore.capabilities = {
      ...(await mockApi.getPlatformCapabilities()),
      platform: 'macos',
      installed_apps: { status: 'available' },
    };

    const rendered = render(StorageView, {
      props: { onSelectCategory: vi.fn() },
    });

    expect(rendered.body).toContain('Applications');
  });

  it('shows the Applications tab when installed_apps is read_only', async () => {
    platformCapabilitiesStore.capabilities = {
      ...(await mockApi.getPlatformCapabilities()),
      platform: 'windows',
      installed_apps: { status: 'read_only', reason: 'Inspection only' },
    };

    const rendered = render(StorageView, {
      props: { onSelectCategory: vi.fn() },
    });

    expect(rendered.body).toContain('Applications');
  });
});
