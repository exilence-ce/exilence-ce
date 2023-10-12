/* eslint-disable @typescript-eslint/no-floating-promises,
                  @typescript-eslint/promise-function-async,
                  @typescript-eslint/return-await */
import { action, computed, makeObservable, observable, runInAction } from 'mobx';
import { persist } from 'mobx-persist';
import moment from 'moment';

class ResourceHandle {
  @persist borrowedAt!: number;
  @persist releasedAt!: number;
  public promise!: Promise<void>;

  private _tmid!: ReturnType<typeof setTimeout>;
  private _cb!: () => void;
  private _resolve!: () => void;
  private _reject!: (reason?: any) => void;

  constructor(millis: number, cb: () => void) {
    if (!millis || !cb) return;
    this.borrowedAt = moment.utc().valueOf();
    this.releasedAt = this.borrowedAt + millis;
    this._cb = cb;

    this.promise = new Promise((resolve, reject) => {
      this._resolve = resolve;
      this._reject = reject;

      this._tmid = setTimeout(() => {
        this._cb();
        this._resolve();
      }, millis);
    });
  }

  public cancel(reason?: any) {
    clearTimeout(this._tmid);
    this._cb();
    this._reject(reason);
  }

  // In case of retry-after
  public regenerate(millis: number) {
    clearTimeout(this._tmid);
    this.borrowedAt = moment.utc().valueOf();
    this.releasedAt = this.borrowedAt + millis;
    this._tmid = setTimeout(() => {
      this._cb();
      this._resolve();
    }, millis);
  }

  // MobX restored
  public restore(cb: () => void) {
    const millis = moment.utc(this.releasedAt).diff(moment.utc());
    if (millis <= 0) return cb();
    this._cb = cb;
    this.promise = new Promise((resolve, reject) => {
      this._resolve = resolve;
      this._reject = reject;

      this._tmid = setTimeout(() => {
        this._cb();
        this._resolve();
      }, millis);
    });
  }
}

export class RateLimiter {
  @persist policy!: string;
  @observable @persist('list', ResourceHandle) stack: ResourceHandle[] = [];
  queue = { value: 0 };
  @persist retryAfter?: number;
  @persist max!: number;
  @persist window!: number;

  private _destroyed = false;
  private DEBUG = true;

  constructor(policy: string, max: number, window: number) {
    makeObservable(this);
    if (policy) this.policy = policy;
    if (max) this.max = max;
    if (window) this.window = window;
  }

  @action
  restore() {
    // // MobX - Restore states
    if (this.retryAfter) {
      this.applyRetryAfter(this.retryAfter);
    } else if (this.stack?.length > 0) {
      for (const handle of this.stack) {
        handle.restore(() => {
          const idx = this.stack.indexOf(handle);
          if (idx !== -1) {
            runInAction(() => {
              this.stack.splice(idx, 1);
              this.DEBUG && console.log('Removed restored stack ' + this.toString());
            });
          }
        });
      }
      this.DEBUG && console.log('Restored stacks ' + this.toString());
    }
  }

  wait(borrow = true) {
    return this._wait(borrow);
  }

  private async _wait(borrow: boolean): Promise<void> {
    if (this._destroyed) throw new Error('RateLimiter is no longer active');

    if (this.isFullyUtilized) {
      this.queue.value++;
      this.DEBUG && console.log('IsFullyUtilized: Before ' + this.toString());
      await this.stack[0].promise;
      this.queue.value--;
      this.DEBUG && console.log('IsFullyUtilized: After ' + this.toString());
      return this._wait(borrow);
    } else {
      if (borrow) {
        this.push();
      }
    }
  }

  @action
  private push(millies = 0, isRetryAfter: boolean = false) {
    const handle = new ResourceHandle(millies || this.window * 1000, () => {
      const idx = this.stack.indexOf(handle);
      if (idx !== -1) {
        runInAction(() => {
          this.stack.splice(idx, 1);
          this.DEBUG && console.log('Removed stack:  ' + this.toString());
        });
      }
      if (isRetryAfter) {
        this.retryAfter = undefined;
      }
    });
    this.stack.push(handle);
    this.DEBUG && console.log('Pushed stack:  ' + this.toString());
  }

  @action
  applyRetryAfter(retryAfter: number) {
    this.retryAfter = retryAfter;
    const duration = moment.utc(this.retryAfter).diff(moment.utc());
    for (const handle of this.stack) {
      handle.regenerate(duration);
    }
    while (!this.isFullyUtilized) {
      this.push(duration, true);
    }
    this.DEBUG && console.warn(`ApplyRetryAfter: ${this.toString()}`);
  }

  static async waitMulti(limiters: Iterable<RateLimiter>): Promise<void> {
    const _limiters = Array.from(limiters);

    // Wait for retryAfter
    await Promise.all(
      _limiters.map((rl) => {
        if (rl.retryAfter) {
          const duration = moment.utc(rl.retryAfter).diff(moment.utc());
          console.log(`Retry after: ${duration / 1000}s - ${rl.toString()}`);
          return new Promise((resolve) => {
            setTimeout(resolve, duration);
          });
        }
        Promise.resolve();
      })
    );

    try {
      await Promise.all(_limiters.map((rl) => rl.wait(false)));
    } catch (e) {
      if (e instanceof Error && e.message === 'RateLimiter is no longer active') {
        return this.waitMulti(limiters);
      } else {
        throw e;
      }
    }

    if (_limiters.every((rl) => !rl.isFullyUtilized)) {
      _limiters.forEach((rl) => {
        rl.wait();
      });
    } else {
      return this.waitMulti(limiters);
    }
  }

  static estimateTime(count: number, limiters: Iterable<RateLimiter>, ignoreState = false): number {
    // NOTE: Cannot handle existing queue in simulation, because
    //       entries in queue can depend on other limiters in `waitMulti` call.
    //       It means that time returned by `estimateTime` will be increased
    //       multiple times between calls until queue is cleared.

    let simulation: Array<{ max: number; window: number; stack: number[] }>;
    {
      const now = Date.now();
      simulation = Array.from(limiters).map((l) => ({
        max: l.max,
        window: l.window,
        stack: ignoreState
          ? []
          : l.stack.map((entry) => entry.releasedAt - now).sort((a, b) => a - b),
      }));
    }

    let total = 0;
    while (count--) {
      while (simulation.some((limit) => limit.stack.length >= limit.max)) {
        const waitTime = simulation.reduce(
          (ms, limit) =>
            limit.stack.length >= limit.max ? Math.max(limit.stack[0] - total, ms) : ms,
          0
        );

        total += waitTime;

        for (const limit of simulation) {
          limit.stack = limit.stack.filter((time) => time > total);
        }
      }

      for (const limit of simulation) {
        limit.stack.push(total + limit.window * 1000);
      }
    }

    return total;
  }

  isEqualLimit(other: { max: number; window: number }) {
    return this.max === other.max && this.window === other.window;
  }

  @computed
  get isFullyUtilized() {
    return !this.available;
  }

  @computed
  get available() {
    return Math.max(this.max - this.stack.length, 0);
  }

  destroy() {
    this._destroyed = true;
    if (this.queue.value) {
      // shortcircuit awaiters in queue
      this.stack[0].cancel(new Error('RateLimiter is no longer active'));
    }
  }

  toString() {
    const duration = this.retryAfter ? moment.utc(this.retryAfter).diff(moment.utc()) / 1000 : 0;
    return `RateLimiter <${this.policy}> <max=${this.max}:window=${this.window}>: (stack=${
      this.stack?.length || 0
    },queue=${this.queue?.value || 0}) - retryAfter=${duration}`;
  }
}
