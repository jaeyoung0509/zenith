import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import Switch from '../lib/components/Switch.svelte';
import Checkbox from '../lib/components/Checkbox.svelte';

describe('Switch component SSR / visual contracts', () => {
  it('renders input with required aria-label and default emerald styling', () => {
    const rendered = render(Switch, {
      props: {
        checked: true,
        ariaLabel: 'Enable AI Assistant Caches',
      },
    });

    expect(rendered.body).toContain('aria-label="Enable AI Assistant Caches"');
    expect(rendered.body).toContain('peer-checked:bg-emerald-500');
    expect(rendered.body).toContain('peer-focus-visible:ring-emerald-500/40');
    expect(rendered.body).toContain('translate-x-4');
  });

  it('renders unchecked and disabled states accurately', () => {
    const rendered = render(Switch, {
      props: {
        checked: false,
        disabled: true,
        ariaLabel: 'Disabled Switch',
      },
    });

    expect(rendered.body).toContain('aria-label="Disabled Switch"');
    expect(rendered.body).toContain('disabled');
    expect(rendered.body).toContain('translate-x-0');
    expect(rendered.body).toContain('opacity-50');
    expect(rendered.body).toContain('cursor-not-allowed');
  });
});

describe('Checkbox component SSR / visual contracts', () => {
  it('renders checked checkbox with emerald styling and white checkmark icon', () => {
    const rendered = render(Checkbox, {
      props: {
        checked: true,
        ariaLabel: 'Select Storage',
      },
    });

    expect(rendered.body).toContain('aria-label="Select Storage"');
    expect(rendered.body).toContain('bg-emerald-500');
    expect(rendered.body).toContain('border-emerald-500');
    expect(rendered.body).toContain('peer-focus-visible:ring-emerald-500/40');
    // Verify lucide Check SVG is rendered
    expect(rendered.body).toContain('<svg');
    expect(rendered.body).toContain('stroke-white');
  });

  it('renders unchecked and disabled checkbox accurately', () => {
    const rendered = render(Checkbox, {
      props: {
        checked: false,
        disabled: true,
        ariaLabel: 'Disabled Checkbox',
      },
    });

    expect(rendered.body).toContain('aria-label="Disabled Checkbox"');
    expect(rendered.body).toContain('disabled');
    expect(rendered.body).toContain('opacity-50');
    expect(rendered.body).toContain('cursor-not-allowed');
    expect(rendered.body).not.toContain('bg-emerald-500');
    expect(rendered.body).not.toContain('<svg');
  });
});
