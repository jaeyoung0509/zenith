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

  it('renders the review-only copy and supported ecosystem guidance', () => {
    const rendered = render(DeveloperArtifactsView, {
      props: { onBack: () => undefined },
    });

    expect(rendered.body).toContain('Developer Artifacts');
    expect(rendered.body).toContain('nothing selected by default');
    expect(rendered.body).toContain('Java/Kotlin');
    expect(rendered.body).toContain('Terraform');
  });
});
