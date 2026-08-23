import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'svelte/server';
import Switch from '../lib/components/Switch.svelte';
import Checkbox from '../lib/components/Checkbox.svelte';
import CategoryCard from '../lib/components/CategoryCard.svelte';
import { scanStore } from '../lib/stores/scan.svelte';

afterEach(() => {
  scanStore.selectedMap = {};
});

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

  it('does not duplicate the default safe subtotal as a Selected metric', () => {
    scanStore.selectedMap = { safe: true, rebuild: false };
    const rendered = render(CategoryCard, {
      props: {
        categoryResult: {
          category: 'developer',
          display_name: 'Developer',
          items: [
            {
              id: 'safe',
              signature_id: 'developer.safe',
              name: 'Safe cache',
              category: 'developer',
              risk: 'safe',
              path: '/tmp/safe',
              size: { logical: 84.9 * 1024 * 1024, allocated: 84.9 * 1024 * 1024 },
              file_count: 1,
              description: 'Safe cache',
              is_selected: true,
              last_modified: 0,
              exists: true,
            },
            {
              id: 'rebuild',
              signature_id: 'developer.rebuild',
              name: 'Rebuild cache',
              category: 'developer',
              risk: 'rebuild',
              path: '/tmp/rebuild',
              size: { logical: 207.5 * 1024 * 1024, allocated: 207.5 * 1024 * 1024 },
              file_count: 1,
              description: 'Rebuild cache',
              is_selected: false,
              last_modified: 0,
              exists: true,
            },
          ],
          total_bytes: 292.4 * 1024 * 1024,
          safe_bytes: 84.9 * 1024 * 1024,
          rebuild_bytes: 207.5 * 1024 * 1024,
          manual_bytes: 0,
        },
      },
    });

    expect(rendered.body).toContain('Safe: 84.9 MB');
    expect(rendered.body).not.toContain('Selected: 84.9 MB');
  });

  it('keeps Selected visible when the selection differs from the safe subtotal', () => {
    scanStore.selectedMap = { safe: false, rebuild: true };
    const rendered = render(CategoryCard, {
      props: {
        categoryResult: {
          category: 'developer',
          display_name: 'Developer',
          items: [
            {
              id: 'safe',
              signature_id: 'developer.safe',
              name: 'Safe cache',
              category: 'developer',
              risk: 'safe',
              path: '/tmp/safe',
              size: { logical: 80 * 1024 * 1024, allocated: 80 * 1024 * 1024 },
              file_count: 1,
              description: 'Safe cache',
              is_selected: false,
              last_modified: 0,
              exists: true,
            },
            {
              id: 'rebuild',
              signature_id: 'developer.rebuild',
              name: 'Rebuild cache',
              category: 'developer',
              risk: 'rebuild',
              path: '/tmp/rebuild',
              size: { logical: 200 * 1024 * 1024, allocated: 200 * 1024 * 1024 },
              file_count: 1,
              description: 'Rebuild cache',
              is_selected: true,
              last_modified: 0,
              exists: true,
            },
          ],
          total_bytes: 280 * 1024 * 1024,
          safe_bytes: 80 * 1024 * 1024,
          rebuild_bytes: 200 * 1024 * 1024,
          manual_bytes: 0,
        },
      },
    });

    expect(rendered.body).toContain('Selected: 200 MB');
  });
});
