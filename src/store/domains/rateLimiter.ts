/* eslint-disable @typescript-eslint/no-floating-promises,
                  @typescript-eslint/promise-function-async,
                  @typescript-eslint/return-await */
import { action, computed, makeObservable, observable } from 'mobx';
import { persist } from 'mobx-persist';
import moment from 'moment';

const DEBUG = false;

export class ResourceHandle {
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

  public get isRunning() {
    // @ts-ignore
    return Boolean(this._tmid && this.promise && this._cb && this._resolve && this._reject);
  }

  public cancel(reason?: any) {
    clearTimeout(this._tmid);
    this._cb();
    this._reject(reason);
  }

  // In case of retry-after
  public regenerate(millis: number) {
    if (!this.isRunning) {
      DEBUG && console.error('Trying to regenerate handle which is not running');
      return;
    }
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
    if (this.stack?.length > 0) {
      let i = 0;
      for (const handle of this.stack) {
        i++;
        DEBUG && console.info(`Restore handle ${i} ` + this.toString());
        handle.restore(() => {
          this.removeStack(handle);
        });
      }
      DEBUG && console.log('Restored stacks ' + this.toString());
    }

    // The ResourceHandle must be bestored before they can regenerate to a new timestamp
    if (this.retryAfter) {
      this.applyRetryAfter(this.retryAfter);
    }
  }

  wait(borrow = true) {
    return this._wait(borrow);
  }

  private async _wait(borrow: boolean): Promise<void> {
    if (this._destroyed) throw new Error('RateLimiter is no longer active');

    if (this.isFullyUtilized) {
      this.queue.value++;
      DEBUG && console.log('IsFullyUtilized ' + this.toString());
      await this.waitHandle();
      this.queue.value--;
      return this._wait(borrow);
    } else {
      if (borrow) {
        this.push();
      }
    }
  }

  private async waitHandle() {
    if (!this.stack[0].isRunning) {
      DEBUG && console.warn('Waiting for an dead stack ' + this.toString());
      this.removeStack(this.stack[0]);
    } else {
      await this.stack[0].promise;
    }
  }

  @action
  private push(millies = 0, isRetryAfter: boolean = false) {
    const handle = new ResourceHandle(millies || this.window * 1000, () => {
      this.removeStack(handle);
      if (isRetryAfter) {
        this.retryAfter = undefined;
      }
    });
    this.stack.push(handle);
    DEBUG && console.log('Pushed stack:  ' + this.toString());
  }

  @action
  private removeStack(handle: ResourceHandle, restored = false) {
    const idx = this.stack.indexOf(handle);
    if (idx !== -1) {
      this.stack.splice(idx, 1);
      DEBUG && console.log(`Removed ${restored ? 'restored ' : ''}stack: ` + this.toString());
    }
  }

  @action
  applyRetryAfter(retryAfter: number) {
    this.retryAfter = retryAfter;
    const duration = moment.utc(this.retryAfter).diff(moment.utc());
    DEBUG && console.warn(`ApplyRetryAfter wait ${duration}: ${this.toString()}`);
    for (const handle of this.stack) {
      handle.regenerate(duration);
    }
    while (!this.isFullyUtilized) {
      this.push(duration, true);
    }
    DEBUG && console.warn(`ApplyRetryAfter: ${this.toString()}`);
  }

  static async waitMulti(limiters: Iterable<RateLimiter>): Promise<void> {
    const _limiters = Array.from(limiters);

    // Wait for retryAfter
    await Promise.all(
      _limiters.map((rl) => {
        if (rl.retryAfter) {
          const duration = moment.utc(rl.retryAfter).diff(moment.utc());
          DEBUG && console.log(`Retry after: ${duration / 1000}s - ${rl.toString()}`);
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

  static limiterWaitInfo(
    requests: number,
    limiters: RateLimiter[],
    ignoreRequestOverflow = false,
    ignoreState = false
  ) {
    const maxLimiter = limiters.slice().sort((a, b) => b.max - a.max)[0];
    const estimatedTime = this.estimateTime(
      requests,
      [maxLimiter],
      ignoreState,
      ignoreRequestOverflow
    );
    const staticEstimatedTime = ignoreRequestOverflow
      ? 0
      : this.estimateStaticTime(requests, limiters);
    return { estimatedTime, staticEstimatedTime };
  }

  static estimateStaticTime(requests: number, limiters: RateLimiter[]) {
    const maxRequests = limiters.map((x) => x.max).sort((a, b) => b - a)[0] || 0;
    const requestOverflow = requests - maxRequests > 0;
    if (!requestOverflow) return 0;
    return this.estimateTime(requests, limiters, true, true);
  }

  static estimateTime(
    requests: number,
    limiters: Iterable<RateLimiter>,
    ignoreRequestOverflow = false,
    ignoreState = false
  ): number {
    // NOTE: Cannot handle existing queue in simulation, because
    //       entries in queue can depend on other limiters in `waitMulti` call.
    //       It means that time returned by `estimateTime` will be increased
    //       multiple times between calls until queue is cleared.

    let simulation: Array<{ max: number; window: number; stack: number[] }>;
    {
      const now = moment.utc().valueOf();
      simulation = Array.from(limiters)
        .slice()
        .sort((a, b) => a.max - b.max)
        .map((l) => ({
          max: l.max,
          window: l.window,
          stack: ignoreState
            ? []
            : l.stack.map((entry) => entry.releasedAt - now).sort((a, b) => a - b),
        }));
      const maxRequests = simulation.map((limit) => limit.max).sort((a, b) => b - a)[0] || 0;
      if (!ignoreRequestOverflow && requests > maxRequests) {
        requests = requests % maxRequests;
      }
    }

    let total = 0;
    while (requests--) {
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
