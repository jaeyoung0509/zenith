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

  it('does not allow incomplete candidates into a cleanup plan', async () => {
    const result = await mockStorageApi.startDeveloperArtifactScan(
      ['workspace-myproject', 'workspace-work'],
      () => undefined
    );
    const incomplete = result.items.find((item) => !item.complete);
    expect(incomplete).toBeDefined();
    await expect(
      mockStorageApi.prepareDeveloperArtifactCleanup(result.scan_id, [incomplete!.id])
    ).rejects.toThrow('Incomplete artifacts');
  });

  it('scans the whole user scope without manually adding project folders', async () => {
    const workspace = await mockStorageApi.registerDeveloperHomeWorkspace();
    const result = await mockStorageApi.startDeveloperArtifactScan([workspace.id], () => undefined);

    expect(workspace.name).toBe('This Mac');
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
    expect(rendered.body).toContain('Scan this Mac');
    expect(rendered.body).toContain('System, credential, media, and app-bundle paths are bypassed');
    expect(rendered.body).toContain('Project source, manifests, lockfiles, and project roots are never cleanup targets');
    expect(rendered.body).toContain('Java/Kotlin');
    expect(rendered.body).toContain('Terraform');
  });
});
