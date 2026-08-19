import { describe, expect, it } from 'vitest';
import { isQuickPanelDismissShortcut, moveOrdered, reorderOrdered, toggleOrdered } from '../lib/utils/quickPanel';

describe('quick panel customization', () => {
  it('never removes the final visible section', () => {
    expect(toggleOrdered(['storage'], 'storage', true)).toEqual(['storage']);
  });

  it('adds disabled entries at the end', () => {
    expect(toggleOrdered(['storage'], 'memory', true)).toEqual(['storage', 'memory']);
  });

  it('moves entries without crossing collection bounds', () => {
    expect(moveOrdered(['storage', 'memory'], 'memory', -1)).toEqual(['memory', 'storage']);
    expect(moveOrdered(['storage', 'memory'], 'storage', -1)).toEqual(['storage', 'memory']);
  });

  it('reorders entries with drag-and-drop', () => {
    expect(reorderOrdered(['a', 'b', 'c', 'd'], 'd', 'b')).toEqual(['a', 'd', 'b', 'c']);
    expect(reorderOrdered(['a', 'b', 'c', 'd'], 'a', 'c')).toEqual(['b', 'c', 'a', 'd']);
    expect(reorderOrdered(['a', 'b'], 'a', 'a')).toEqual(['a', 'b']);
    expect(reorderOrdered(['a', 'b'], 'unknown', 'a')).toEqual(['a', 'b']);
  });

  it('recognizes Escape and Cmd+W as dismiss shortcuts', () => {
    expect(isQuickPanelDismissShortcut('Escape', false)).toBe(true);
    expect(isQuickPanelDismissShortcut('w', true)).toBe(true);
    expect(isQuickPanelDismissShortcut('w', false)).toBe(false);
  });
});
