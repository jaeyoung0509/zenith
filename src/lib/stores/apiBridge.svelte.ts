import { apiBridgeSnapshot, subscribeApiBridge, type ApiBridgeSnapshot } from '../api';

/**
 * Reactive view of the native/preview data source decision. The bridge itself
 * resolves lazily per call; this store only mirrors the published transitions
 * so the interface can show that it is running on preview data.
 *
 * Subscription is explicit and reference counted: the dashboard and the
 * persistent quick panel each own one, and the first one to leave must not
 * silence the other.
 */
export class ApiBridgeStore {
  snapshot = $state<ApiBridgeSnapshot>(apiBridgeSnapshot());
  private subscriberCount = 0;
  private unsubscribe: (() => void) | null = null;

  subscribe(): () => void {
    this.subscriberCount += 1;
    if (this.subscriberCount === 1) {
      this.snapshot = apiBridgeSnapshot();
      this.unsubscribe = subscribeApiBridge((snapshot) => {
        this.snapshot = snapshot;
      });
    }

    let active = true;
    return () => {
      if (!active) return;
      active = false;
      this.subscriberCount -= 1;
      if (this.subscriberCount === 0) {
        this.unsubscribe?.();
        this.unsubscribe = null;
      }
    };
  }
}

export const apiBridgeStore = new ApiBridgeStore();
