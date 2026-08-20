import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import Checkbox from '../lib/components/Checkbox.svelte';
import Switch from '../lib/components/Switch.svelte';

describe('selection controls state and visual contracts', () => {
  it('renders the switch with semantic enabled styling and an accessible name', () => {
    const { body } = render(Switch, { props: {
      checked: true,
      disabled: false,
      ariaLabel: 'AI Assistant Caches & Logs',
    } });

    expect(body).toContain('aria-label="AI Assistant Caches &amp; Logs"');
    expect(body).toContain('checked');
    expect(body).toContain('peer-checked:bg-emerald-500');
    expect(body).toContain('translate-x-4');
  });

  it('renders the checkbox checked state and required accessible name', () => {
    const { body } = render(Checkbox, { props: {
      checked: true,
      disabled: false,
      ariaLabel: 'Show Storage in sidebar',
    } });

    expect(body).toContain('aria-label="Show Storage in sidebar"');
    expect(body).toContain('checked');
    expect(body).toContain('bg-emerald-500');
    expect(body).toContain('stroke-white');
  });
});
