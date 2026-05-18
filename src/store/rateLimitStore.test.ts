import { RateLimitStore } from './rateLimitStore';
import { RootStore } from './rootStore';

describe('RateLimitStore retry-after handling', () => {
  it('stores retry-after as an expiring timestamp', () => {
    const store = new RateLimitStore({} as RootStore);

    store.setRetryAfter(2);

    expect(store.isRetryAfterActive()).toBe(true);
    expect(store.getRetryAfterMillisecondsRemaining()).toBeGreaterThan(0);
    expect(store.getRetryAfterMillisecondsRemaining()).toBeLessThanOrEqual(2000);
  });

  it('clears non-positive retry-after values', () => {
    const store = new RateLimitStore({} as RootStore);

    store.setRetryAfter(2);
    store.setRetryAfter(0);

    expect(store.isRetryAfterActive()).toBe(false);
    expect(store.getRetryAfterMillisecondsRemaining()).toBe(0);
  });
});
