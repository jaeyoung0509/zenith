/**
 * Uncaught frontend failures.
 *
 * A rejected event handler or a failed top-level await used to end in the
 * developer console only: the interface stayed silent, and the diagnostics the
 * user could send had no trace of it. The window listeners installed in
 * `main.ts` record the message here so the Diagnostics section can show it.
 */

export type FrontendErrorKind = 'error' | 'rejection';

export interface FrontendErrorEntry {
  kind: FrontendErrorKind;
  message: string;
  at: string;
}

/** Newest entries are kept; older ones fall off the front. */
const ENTRY_CAP = 20;

function describeRejection(reason: unknown): string {
  if (reason instanceof Error) {
    return reason.message || reason.name;
  }
  if (typeof reason === 'string') {
    return reason;
  }
  try {
    return JSON.stringify(reason) ?? String(reason);
  } catch {
    return String(reason);
  }
}

export class FrontendErrorStore {
  entries = $state<FrontendErrorEntry[]>([]);

  push(kind: FrontendErrorKind, message: string) {
    const entry: FrontendErrorEntry = {
      kind,
      message: message.trim() || 'No message was provided',
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
    const onError = (event: ErrorEvent) => {
      this.push('error', event.message || String(event.error ?? 'Unknown error'));
    };
    const onRejection = (event: PromiseRejectionEvent) => {
      this.push('rejection', describeRejection(event.reason));
    };

    target.addEventListener('error', onError);
    target.addEventListener('unhandledrejection', onRejection);

    return () => {
      target.removeEventListener('error', onError);
      target.removeEventListener('unhandledrejection', onRejection);
    };
  }
}

export const frontendErrorStore = new FrontendErrorStore();
