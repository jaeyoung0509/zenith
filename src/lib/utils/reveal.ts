import { platformCapabilitiesStore } from '../stores/platformCapabilities.svelte';

const REVEAL_UNAVAILABLE_FALLBACK = 'Revealing files is unavailable on this platform.';

/** Reveals are a native `system_actions` feature, not a webview capability. */
export function canReveal(): boolean {
  return platformCapabilitiesStore.isInspectable('system_actions');
}

export function revealUnavailableReason(): string {
  return (
    platformCapabilitiesStore.feature('system_actions')?.reason ?? REVEAL_UNAVAILABLE_FALLBACK
  );
}

/**
 * Runs a reveal for a control that is already gated by `canReveal()`.
 * Rejections are handed to the caller's inline error state instead of being
 * swallowed by a floating promise.
 */
export async function runReveal(
  reveal: () => Promise<void>,
  reportError: (message: string) => void
): Promise<void> {
  if (!canReveal()) return;
  try {
    await reveal();
  } catch (cause) {
    reportError(cause instanceof Error ? cause.message : String(cause));
  }
}
