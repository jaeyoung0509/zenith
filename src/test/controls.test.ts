import { describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import Switch from '../lib/components/Switch.svelte';
import Checkbox from '../lib/components/Checkbox.svelte';
import CategoryCard from '../lib/components/CategoryCard.svelte';

describe('Switch component SSR / visual contracts', () => {
  it('renders input with required aria-label and default emerald styling', () => {
    const rendered = render(Switch, {
      props: {
        checked: true,
        ariaLabel: 'Enable AI Assistant Caches',
      },
    });

    expect(rendered.body).toContain('aria-label="Enable AI Assistant Caches"');
    expect(rendered.body).toContain('peer-checked:bg-success');
    expect(rendered.body).toContain('peer-focus-visible:ring-success/40');
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
    expect(rendered.body).toContain('bg-success');
    expect(rendered.body).toContain('border-success');
    expect(rendered.body).toContain('peer-focus-visible:ring-success/40');
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
    expect(rendered.body).not.toContain('bg-success');
    expect(rendered.body).not.toContain('<svg');
  });
});

describe('metric and action consistency contracts', () => {
  it('reserves a stable no-wrap column for category byte metrics', () => {
    const rendered = render(CategoryCard, {
      props: {
        categoryResult: {
          category: 'developer',
          display_name: 'Developer',
          items: [],
          total_bytes: 198.8 * 1024 * 1024,
          safe_bytes: 0,
          rebuild_bytes: 198.8 * 1024 * 1024,
          manual_bytes: 0,
        },
      },
    });

    expect(rendered.body).toContain('w-[7rem]');
    expect(rendered.body).toContain('whitespace-nowrap');
    expect(rendered.body).toContain('Rebuild: 198.8 MB');
  });
});
