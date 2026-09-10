/**
 * Focus management utilities for accessible dialogs, overlays, and view transitions.
 */

export function isFocusable(element: unknown): element is HTMLElement {
  if (typeof HTMLElement === 'undefined') return false;
  if (!(element instanceof HTMLElement)) return false;
  if (!element.isConnected) return false;
  if ('disabled' in element && Boolean((element as HTMLButtonElement | HTMLInputElement).disabled)) {
    return false;
  }
  if (element.getAttribute('aria-disabled') === 'true') return false;
  if (element.closest('[hidden], [inert], [aria-hidden="true"]')) return false;
  if (typeof getComputedStyle === 'function') {
    for (let current: HTMLElement | null = element; current; current = current.parentElement) {
      const style = getComputedStyle(current);
      if (style.display === 'none' || style.visibility === 'hidden') return false;
    }
  }

  const naturallyFocusable = element.matches(
    'button:not(:disabled), a[href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), summary, iframe, [contenteditable="true"]'
  );
  return naturallyFocusable || (element.hasAttribute('tabindex') && element.tabIndex >= 0);
}

export function findStableFocusTarget(
  preferredTarget?: HTMLElement | null,
  fallbackContainer?: HTMLElement | null,
): HTMLElement | null {
  if (isFocusable(preferredTarget)) {
    return preferredTarget;
  }

  // 1. Storage header scan control
  const scanButton = document.getElementById('storage-scan-button');
  if (isFocusable(scanButton)) {
    return scanButton;
  }

  // 2. Active tab in segmented tab list
  const activeTab = document.querySelector('[role="tab"][aria-selected="true"]');
  if (isFocusable(activeTab)) {
    return activeTab as HTMLElement;
  }

  // 3. Any enabled button or focusable control inside the container or active tabpanel
  const container = fallbackContainer ?? document.querySelector('[role="tabpanel"]');
  if (container instanceof HTMLElement) {
    const focusable = container.querySelector(
      'button:not([disabled]), [role="tab"]:not([disabled]), [tabindex="0"]:not([role="tabpanel"])'
    );
    if (isFocusable(focusable)) {
      return focusable as HTMLElement;
    }
  }

  return null;
}

export function restoreFocus(
  preferredTarget?: HTMLElement | null,
  fallbackContainer?: HTMLElement | null,
): boolean {
  const target = findStableFocusTarget(preferredTarget, fallbackContainer);
  if (target) {
    target.focus();
    return true;
  }
  return false;
}
