import { describe, expect, it, vi } from 'vitest';
import { render } from 'svelte/server';
import ApplicationsView from '../routes/dashboard/ApplicationsView.svelte';

describe('ApplicationsView responsive layout contract', () => {
  it('bounds the inventory and reserves a desktop detail pane at the 960px baseline', () => {
    const rendered = render(ApplicationsView, {
      props: {
        onBack: vi.fn(),
      },
    });

    expect(rendered.body).toContain(
      'md:grid-cols-[minmax(220px,0.85fr)_minmax(0,1.15fr)]'
    );
    expect(rendered.body).toContain('max-h-[calc(100vh-245px)]');
    expect(rendered.body).toContain('md:max-h-[calc(100vh-5rem)]');
    expect(rendered.body).toContain('Choose an application');
  });
});
