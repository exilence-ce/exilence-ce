import moment from 'moment';

class ResourceHandle {
  borrowedAt: number;
  releasedAt: number;

  constructor(millis: number) {
    this.borrowedAt = moment.utc().valueOf();
    this.releasedAt = this.borrowedAt + millis;
  }
}

export class RateLimiter {
  stack: ResourceHandle[] = [];
  queue = { value: 0 };
  retryAfter?: number;
  constructor(public max: number, public window: number) {}

  // Should be less than defined window
  push(windowSec: number) {
    const waittime = windowSec < this.window ? windowSec : this.window;
    const handle = new ResourceHandle(waittime * 1000);
    this.stack.push(handle);
    // console.log(`Add stack waittime ${windowSec} <stack=${this.stack.length}>`);
  }
}

export function estimateTime(
  count: number,
  limiters: Iterable<RateLimiter>,
  ignoreState = false
): number {
  // NOTE: Cannot handle existing queue in simulation, because
  //       entries in queue can depend on other limiters in `waitMulti` call.
  //       It means that time returned by `estimateTime` will be increased
  //       multiple times between calls until queue is cleared.

  let simulation: Array<{ max: number; window: number; stack: number[] }>;
  {
    const now = moment.utc().valueOf();
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
      const waitTime = simulation.reduce((ms, limit) => {
        return limit.stack.length >= limit.max ? Math.max(limit.stack[0] - total, ms) : ms;
      }, 0);

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

test('Estimated Time', async () => {
  const stashLimiter1 = new RateLimiter(10, 5);
  const stashLimiter2 = new RateLimiter(15, 150);

  stashLimiter1.push(1);
  stashLimiter1.push(1.2);
  stashLimiter1.push(1.4);
  stashLimiter1.push(1.6);
  stashLimiter1.push(2);
  stashLimiter1.push(2.2);
  stashLimiter1.push(2.4);
  stashLimiter1.push(2.6);
  stashLimiter1.push(3);
  stashLimiter1.push(5);

  // stashLimiter2.push(10);
  // stashLimiter2.push(10.2);
  // stashLimiter2.push(10.4);
  // stashLimiter2.push(11);
  stashLimiter2.push(11.2);
  stashLimiter2.push(11.4);
  stashLimiter2.push(11.6);
  stashLimiter2.push(12);
  stashLimiter2.push(12.2);
  stashLimiter2.push(12.4);
  stashLimiter2.push(12.6);
  stashLimiter2.push(13);
  stashLimiter2.push(13.2);
  stashLimiter2.push(14);
  stashLimiter2.push(15);

  const limiters = [stashLimiter1, stashLimiter2];

  const estimatedTime = estimateTime(1, limiters);
  console.log(`Estimated time ${estimatedTime / 1000} s`);
});

// 11399
