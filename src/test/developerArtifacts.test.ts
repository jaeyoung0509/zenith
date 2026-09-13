import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import DeveloperArtifactsView from '../routes/dashboard/DeveloperArtifactsView.svelte';
import { mockStorageApi } from '../lib/api/storage';

describe('developer artifact review workflow', () => {
  it('streams marker-backed candidates with empty default selection', async () => {
    const events: string[] = [];
    const result = await mockStorageApi.startDeveloperArtifactScan(['workspace-myproject'], (event) => {
      events.push(event.type);
    });

    expect(events).toContain('artifact_found');
    expect(result.items.length).toBeGreaterThan(0);
    expect(result.items.every((item) => item.selected_by_default === false)).toBe(true);
    expect(result.items.some((item) => item.ecosystem === 'rust')).toBe(true);
  });

  it('allows measurement-incomplete candidates only through explicit selection', async () => {
    const result = await mockStorageApi.startDeveloperArtifactScan(
      ['workspace-myproject', 'workspace-work'],
      () => undefined
    );
    const incomplete = result.items.find((item) => item.status === 'measurement_incomplete');
    expect(incomplete).toBeDefined();
    await expect(
      mockStorageApi.prepareDeveloperArtifactCleanup(result.scan_id, [incomplete!.id])
    ).resolves.toMatchObject({ item_count: 1 });
  });

  it('keeps safety-blocked candidates out of cleanup plans', async () => {
    const result = await mockStorageApi.startDeveloperArtifactScan(['workspace-work'], () => undefined);
    const blocked = result.items.find((item) => item.status === 'safety_blocked');
    expect(blocked).toBeDefined();
    await expect(
      mockStorageApi.prepareDeveloperArtifactCleanup(result.scan_id, [blocked!.id])
    ).rejects.toThrow('safety checks');
  });

  it('rejects forged artifact IDs even when a valid ID is also selected', async () => {
    const result = await mockStorageApi.startDeveloperArtifactScan(['workspace-myproject'], () => undefined);
    const valid = result.items.find((item) => item.status === 'complete');
    expect(valid).toBeDefined();
    await expect(
      mockStorageApi.prepareDeveloperArtifactCleanup(result.scan_id, [valid!.id, 'forged-artifact'])
    ).rejects.toThrow('inventory changed');
  });

  it('scans the whole user scope without manually adding project folders', async () => {
    const workspace = await mockStorageApi.registerDeveloperHomeWorkspace();
    const result = await mockStorageApi.startDeveloperArtifactScan([workspace.id], () => undefined);

    expect(workspace.name).toBe('This Computer');
    expect(result.items.length).toBeGreaterThan(3);
    expect(result.items.every((item) => item.workspace_id === workspace.id)).toBe(true);
    expect(result.items.some((item) => item.ecosystem === 'kotlin')).toBe(true);
  });

  it('renders the review-only copy and supported ecosystem guidance', () => {
    const rendered = render(DeveloperArtifactsView, {
      props: { onBack: () => undefined },
    });

    expect(rendered.body).toContain('Developer Artifacts');
    expect(rendered.body).toContain('nothing selected by default');
    expect(rendered.body).toContain('Scan this computer');
    expect(rendered.body).toContain('System, credential, media, and installed-application paths are bypassed');
    expect(rendered.body).toContain('Project source, manifests, lockfiles, and project roots are never cleanup targets');
    expect(rendered.body).toContain('Java/Kotlin');
    expect(rendered.body).toContain('Terraform');
    expect(rendered.body).not.toContain('Incomplete · blocked');
  });

  it('reports the gated Downloads folder as uninspected in the whole-home preview', async () => {
    const workspace = await mockStorageApi.registerDeveloperHomeWorkspace();
    const events: string[] = [];
    const result = await mockStorageApi.startDeveloperArtifactScan([workspace.id], (event) => {
      events.push(event.type === 'uninspected' ? `uninspected:${event.name}` : event.type);
    });

    expect(result.uninspected).toMatchObject([
      { name: 'Downloads', path: '/Users/mock/Downloads', reason: 'permission_denied', retryable: true },
    ]);
    expect(result.skipped_entries).toBeGreaterThan(0);
    expect(events).toContain('uninspected:Downloads');
  });

  it('renders a refused folder notice with its reason and an actionable rescan', () => {
    const rendered = render(DeveloperArtifactsView, {
      props: {
        onBack: () => undefined,
        initialResult: {
          scan_id: 'mock-developer-scan-downloads',
          items: [],
          discovered_count: 0,
          measured_count: 0,
          skipped_entries: 1,
          cancelled: false,
          truncated: false,
          uninspected: [
            {
              path: '/Users/mock/Downloads',
              name: 'Downloads',
              reason: 'permission_denied',
              retryable: true,
            },
          ],
        },
      },
    });

    expect(rendered.body).toContain('role="status"');
    expect(rendered.body).toContain('data-testid="developer-artifact-uninspected-notice"');
    expect(rendered.body).toContain('Downloads');
    expect(rendered.body).toContain('/Users/mock/Downloads');
    expect(rendered.body).toContain('The operating system refused access, so this folder was not inspected.');
    expect(rendered.body).toContain('Permission refused · retry allowed');
    expect(rendered.body).toContain('System Settings');
    expect(rendered.body).toContain('Files and Folders');
    expect(rendered.body).toContain('Scan again');
    expect(rendered.body).toContain('1 uninspected');
    expect(rendered.body).toContain('Result is partial');
  });

  it('omits the permission guidance for an unreadable folder that a retry cannot include', () => {
    const rendered = render(DeveloperArtifactsView, {
      props: {
        onBack: () => undefined,
        initialResult: {
          scan_id: 'mock-developer-scan-unreadable',
          items: [],
          discovered_count: 0,
          measured_count: 0,
          skipped_entries: 1,
          cancelled: false,
          truncated: false,
          uninspected: [
            {
              path: '/Users/mock/work/broken',
              name: 'broken',
              reason: 'unreadable',
              retryable: false,
            },
          ],
        },
      },
    });

    expect(rendered.body).toContain('data-testid="developer-artifact-uninspected-notice"');
    expect(rendered.body).toContain('This folder could not be read, so it was not inspected.');
    expect(rendered.body).toContain('Unreadable');
    expect(rendered.body).not.toContain('System Settings');
    expect(rendered.body).not.toContain('Permission refused');
  });
});
