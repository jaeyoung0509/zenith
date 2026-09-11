import { afterEach, describe, expect, it, vi } from 'vitest';
import { render } from 'svelte/server';
import ApplicationsView from '../routes/dashboard/ApplicationsView.svelte';
import { platformCapabilitiesStore } from '../lib/stores/platformCapabilities.svelte';
import { mockApi } from '../lib/api/mock';

describe('ApplicationsView responsive layout contract', () => {
  afterEach(() => platformCapabilitiesStore.reset());

  it('explains Windows inspection-only support before an app is selected', async () => {
    platformCapabilitiesStore.capabilities = {
      ...await mockApi.getPlatformCapabilities(),
      platform: 'windows',
      app_uninstall: { status: 'unavailable', reason: 'Use Windows Settings to uninstall applications.' },
    };
    const { body } = render(ApplicationsView, { props: { onBack: vi.fn() } });
    expect(body).toContain('Use Windows Settings to uninstall applications.');
    expect(body).not.toContain('Moves to Trash, never permanently deletes');
    expect(body).not.toContain('/Applications and ~/Applications');
  });

  it('explains installed applications inventory is unavailable on Windows', async () => {
    platformCapabilitiesStore.capabilities = {
      ...await mockApi.getPlatformCapabilities(),
      platform: 'windows',
      installed_apps: { status: 'unavailable', reason: 'Windows application inventory is not supported. Use Windows Settings.' },
      app_uninstall: { status: 'unavailable', reason: 'Use Windows Settings to uninstall applications.' },
    };
    const { body } = render(ApplicationsView, { props: { onBack: vi.fn() } });
    expect(body).toContain('Applications unavailable');
    expect(body).toContain('Windows application inventory is not supported. Use Windows Settings.');
  });
  it('bounds the inventory and reserves a desktop detail pane at the 960px baseline', () => {
    const rendered = render(ApplicationsView, {
      props: {
        onBack: vi.fn(),
      },
    });

    expect(rendered.body).toContain(
      'md:grid-cols-[minmax(220px,1fr)_minmax(0,1.1fr)]'
    );
    expect(rendered.body).toContain('max-h-[calc(100vh-245px)]');
    expect(rendered.body).toContain('md:max-h-[calc(100vh-5rem)]');
    expect(rendered.body).toContain('Choose an application');
  });
});
