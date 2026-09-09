/** @vitest-environment jsdom */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { handleSegmentedTabKeydown } from '../lib/utils/segmentedTabs';

afterEach(() => {
  document.body.replaceChildren();
});

function pressKey(target: HTMLElement, key: string) {
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }));
}

describe('SegmentedTabs keyboard interaction', () => {
  it('selects and focuses tabs with Arrow, End, and Home key events', () => {
    const tabItems = [
      { id: 'cleanup' },
      { id: 'artifacts' },
      { id: 'disks' },
    ];
    const onSelect = vi.fn();
    const tablist = document.createElement('div');
    tablist.setAttribute('role', 'tablist');
    const tabs = tabItems.map((_, index) => {
      const button = document.createElement('button');
      button.setAttribute('role', 'tab');
      button.addEventListener('keydown', (event) => {
        handleSegmentedTabKeydown(event, index, tabItems, tablist, onSelect);
      });
      tablist.append(button);
      return button;
    });
    document.body.append(tablist);

    expect(tabs).toHaveLength(3);

    tabs[0].focus();
    const arrowRight = new KeyboardEvent('keydown', { key: 'ArrowRight', bubbles: true, cancelable: true });
    tabs[0].dispatchEvent(arrowRight);
    expect(arrowRight.defaultPrevented).toBe(true);
    expect(onSelect).toHaveBeenLastCalledWith('artifacts');
    expect(document.activeElement).toBe(tabs[1]);

    pressKey(tabs[1], 'End');
    expect(onSelect).toHaveBeenLastCalledWith('disks');
    expect(document.activeElement).toBe(tabs[2]);

    pressKey(tabs[2], 'Home');
    expect(onSelect).toHaveBeenLastCalledWith('cleanup');
    expect(document.activeElement).toBe(tabs[0]);

    pressKey(tabs[0], 'ArrowLeft');
    expect(onSelect).toHaveBeenLastCalledWith('disks');
    expect(document.activeElement).toBe(tabs[2]);
  });
});
