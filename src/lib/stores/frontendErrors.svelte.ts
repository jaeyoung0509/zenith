/**
 * Uncaught frontend failures.
 *
 * A rejected event handler or a failed top-level await used to end in the
 * developer console only: the interface stayed silent. The window listeners
 * installed in `main.ts` record safe event metadata for the Diagnostics view.
 * Error messages and rejection reasons can contain credentials or local paths,
 * so they must never enter this user-facing store.
 */

export type FrontendErrorKind = 'error' | 'rejection';

export interface FrontendErrorEntry {
  id: number;
  kind: FrontendErrorKind;
  message: string;
  at: string;
}

/** Newest entries are kept; older ones fall off the front. */
const ENTRY_CAP = 20;
const SAFE_MESSAGES: Record<FrontendErrorKind, string> = {
  error: 'The interface encountered an unexpected error.',
  rejection: 'A background operation failed unexpectedly.',
};

export class FrontendErrorStore {
  entries = $state<FrontendErrorEntry[]>([]);
  private nextId = 0;

  push(kind: FrontendErrorKind) {
    const entry: FrontendErrorEntry = {
      id: this.nextId++,
      kind,
      message: SAFE_MESSAGES[kind],
      at: new Date().toISOString(),
    };
    this.entries = [...this.entries, entry].slice(-ENTRY_CAP);
  }

  clear() {
    this.entries = [];
  }

  /**
   * Records uncaught errors and unhandled rejections raised by `target`.
   * Returns the removal function, so a caller can install exactly one capture.
   */
  captureFrom(target: Window): () => void {
    const onError = () => this.push('error');
    const onRejection = () => this.push('rejection');

    target.addEventListener('error', onError);
    target.addEventListener('unhandledrejection', onRejection);

    return () => {
      target.removeEventListener('error', onError);
      target.removeEventListener('unhandledrejection', onRejection);
    };
  }
}

export const frontendErrorStore = new FrontendErrorStore();
