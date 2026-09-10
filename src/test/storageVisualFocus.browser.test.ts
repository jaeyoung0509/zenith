/** @vitest-environment jsdom */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { isFocusable, findStableFocusTarget, restoreFocus } from '../lib/utils/focus';

describe('Storage visual focus and modal interaction semantics', () => {
  beforeEach(() => {
    document.body.replaceChildren();
  });

  afterEach(() => {
    document.body.replaceChildren();
  });

  it('correctly filters focusable elements based on connection, disabled, hidden, and aria-hidden', () => {
    const button = document.createElement('button');
    expect(isFocusable(button)).toBe(false);

    document.body.append(button);
    expect(isFocusable(button)).toBe(true);

    button.disabled = true;
    expect(isFocusable(button)).toBe(false);

    button.disabled = false;
    button.setAttribute('aria-hidden', 'true');
    expect(isFocusable(button)).toBe(false);

    button.removeAttribute('aria-hidden');
    button.setAttribute('hidden', '');
    expect(isFocusable(button)).toBe(false);

    button.removeAttribute('hidden');
    expect(isFocusable(button)).toBe(true);
  });

  it('restores focus to preferredTarget when connected and enabled', () => {
    const trigger = document.createElement('button');
    document.body.append(trigger);

    expect(restoreFocus(trigger)).toBe(true);
    expect(document.activeElement).toBe(trigger);
  });

  it('falls back to storage-scan-button when preferredTarget becomes disabled during cleanup', () => {
    const reviewCleanupTrigger = document.createElement('button');
    reviewCleanupTrigger.disabled = true; // disabled as cleanup runs

    const scanButton = document.createElement('button');
    scanButton.id = 'storage-scan-button';

    document.body.append(reviewCleanupTrigger, scanButton);

    const target = findStableFocusTarget(reviewCleanupTrigger);
    expect(target).toBe(scanButton);

    const success = restoreFocus(reviewCleanupTrigger);
    expect(success).toBe(true);
    expect(document.activeElement).toBe(scanButton);
  });

  it('falls back to active tab when scan button is disabled or missing', () => {
    const reviewCleanupTrigger = document.createElement('button');
    reviewCleanupTrigger.disabled = true;

    const activeTab = document.createElement('button');
    activeTab.setAttribute('role', 'tab');
    activeTab.setAttribute('aria-selected', 'true');

    document.body.append(reviewCleanupTrigger, activeTab);

    const target = findStableFocusTarget(reviewCleanupTrigger);
    expect(target).toBe(activeTab);

    expect(restoreFocus(reviewCleanupTrigger)).toBe(true);
    expect(document.activeElement).toBe(activeTab);
  });

  it('falls back to first enabled button in tabpanel when header controls are unavailable', () => {
    const tabpanel = document.createElement('div');
    tabpanel.setAttribute('role', 'tabpanel');

    const panelAction = document.createElement('button');
    tabpanel.append(panelAction);
    document.body.append(tabpanel);

    const target = findStableFocusTarget(null, tabpanel);
    expect(target).toBe(panelAction);

    expect(restoreFocus(null, tabpanel)).toBe(true);
    expect(document.activeElement).toBe(panelAction);
  });

  it('simulates dialog focus trapping on Tab and Shift+Tab', () => {
    const dialog = document.createElement('dialog');
    const closeBtn = document.createElement('button');
    const doneBtn = document.createElement('button');
    dialog.append(closeBtn, doneBtn);
    document.body.append(dialog);

    function handleKeydown(event: KeyboardEvent) {
      if (event.key === 'Tab') {
        const focusable = dialog.querySelectorAll<HTMLElement>(
          'button:not([disabled]), [tabindex]:not([tabindex="-1"])'
        );
        if (focusable.length === 0) return;
        const first = focusable[0];
        const last = focusable[focusable.length - 1];
        if (event.shiftKey && document.activeElement === first) {
          event.preventDefault();
          last.focus();
        } else if (!event.shiftKey && document.activeElement === last) {
          event.preventDefault();
          first.focus();
        }
      }
    }

    closeBtn.focus();
    expect(document.activeElement).toBe(closeBtn);

    // Tab forwards from last button wraps to first
    doneBtn.focus();
    const tabEvent = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    handleKeydown(tabEvent);
    expect(tabEvent.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(closeBtn);

    // Shift+Tab backwards from first button wraps to last
    const shiftTabEvent = new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true, cancelable: true });
    handleKeydown(shiftTabEvent);
    expect(shiftTabEvent.defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(doneBtn);
  });

  it('handles Escape key by closing modal and restoring focus', () => {
    const scanButton = document.createElement('button');
    scanButton.id = 'storage-scan-button';
    document.body.append(scanButton);

    const onClose = vi.fn(() => {
      restoreFocus(null);
    });

    const dialog = document.createElement('dialog');
    const doneBtn = document.createElement('button');
    dialog.append(doneBtn);
    document.body.append(dialog);
    doneBtn.focus();

    function handleKeydown(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
      }
    }

    const escEvent = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    handleKeydown(escEvent);

    expect(escEvent.defaultPrevented).toBe(true);
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(scanButton);
  });
});
