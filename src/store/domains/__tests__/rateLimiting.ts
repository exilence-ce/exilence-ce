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
  push(windowSec: number = this.window) {
    const waittime = windowSec < this.window ? windowSec : this.window;
    const handle = new ResourceHandle(waittime * 1000);
    this.stack.push(handle);
    // console.log(`Add stack waittime ${windowSec} <stack=${this.stack.length}>`);
  }
  toString() {
    return `<max=${this.max}:window=${this.window}>: (stack=${this.stack?.length || 0})`;
  }
}

function limiterWaitInfo(
  requests: number,
  limiters: RateLimiter[],
  ignoreRequestOverflow = false,
  ignoreState = false
) {
  //     // 2:10:60,5:300:300 => 300 + 15
  //     // 10:15:60,30:60:300 =>
  //     // 15:10:60,30:300:300 =>
  //     // 5:5:10,10:10:30,15:10:300
  //     // 5:10:60,30:300:300
  const maxLimiter = limiters.slice().sort((a, b) => b.max - a.max)[0];
  const estimatedTime = estimateTime(requests, [maxLimiter], ignoreState, ignoreRequestOverflow);
  const staticEstimatedTime = ignoreRequestOverflow ? 0 : estimateStaticTime(requests, limiters);
  return { estimatedTime, staticEstimatedTime };
}

function estimateStaticTime(requests: number, limiters: RateLimiter[]) {
  const maxRequests = limiters.map((x) => x.max).sort((a, b) => b - a)[0] || 0;
  const requestOverflow = requests - maxRequests > 0;
  if (!requestOverflow) return 0;
  return estimateTime(requests, limiters, true, true);
}

// TODO: Calculate future requests
// function estimateTime(
//   requests: number,
//   limiters: Iterable<RateLimiter>,
//   ignoreRequestOverflow = false,
//   ignoreState = false,
//   calcFuture = { requestsToAddBeforeRef: 0, addedRefTime: 0 }
// ): number {
//   // NOTE: Cannot handle existing queue in simulation, because
//   //       entries in queue can depend on other limiters in `waitMulti` call.
//   //       It means that time returned by `estimateTime` will be increased
//   //       multiple times between calls until queue is cleared.

//   let simulation: Array<{ max: number; window: number; stack: number[] }>;
//   {
//     const nowRef = moment.utc();
//     const now = moment.utc().add(calcFuture.addedRefTime, 'milliseconds').valueOf();
//     simulation = Array.from(limiters)
//       .slice()
//       .sort((a, b) => a.max - b.max)
//       .map((l) => ({
//         max: l.max,
//         window: l.window,
//         stack: ignoreState
//           ? []
//           : l.stack
//               .concat(
//                 Array.from(Array(calcFuture.requestsToAddBeforeRef)).map(() => {
//                   return {
//                     borrowedAt: nowRef.valueOf(),
//                     releasedAt: nowRef.valueOf() + l.window * 1000,
//                   };
//                 })
//               )
//               .map((entry) => entry.releasedAt - now)
//               .sort((a, b) => a - b),
//       }));
//     console.log(simulation);
//     const maxRequests = simulation.map((limit) => limit.max).sort((a, b) => b - a)[0] || 0;
//     if (!ignoreRequestOverflow && requests > maxRequests) {
//       requests = requests % maxRequests;
//     }
//   }

//   let total = 0;
//   while (requests--) {
//     while (simulation.some((limit) => limit.stack.length >= limit.max)) {
//       const waitTime = simulation.reduce(
//         (ms, limit) =>
//           limit.stack.length >= limit.max ? Math.max(limit.stack[0] - total, ms) : ms,
//         0
//       );

//       total += waitTime;

//       for (const limit of simulation) {
//         limit.stack = limit.stack.filter((time) => time > total);
//       }
//     }

//     for (const limit of simulation) {
//       limit.stack.push(total + limit.window * 1000);
//     }
//   }

//   return total;
// }

function estimateTime(
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

// 2:10:60,5:300:300 => 300 + 15
// 10:15:60,30:60:300 =>

// 5:5:10,10:10:30,15:10:300
// 5:10:60,30:300:300

// 15:10, 30:300 => 16 stacks => 15sec

// stashLimiter1 = new RateLimiter(15, 10);
// stashLimiter2 = new RateLimiter(30, 300);

// stashLimiter1.push(2);
// stashLimiter1.push(2);
// stashLimiter1.push(1.4);
// stashLimiter1.push(1.6);
// stashLimiter1.push(2);
// stashLimiter1.push(2.2);
// stashLimiter1.push(2.4);
// stashLimiter1.push(2.6);
// stashLimiter1.push(3);
// stashLimiter1.push(5);

// stashLimiter2.push(6);
// stashLimiter2.push(6);
// stashLimiter2.push(10.4);
// stashLimiter2.push(11);
// stashLimiter2.push(11.2);
// stashLimiter2.push(11.4);
// stashLimiter2.push(11.6);
// stashLimiter2.push(12);
// stashLimiter2.push(12.2);
// stashLimiter2.push(12.4);
// stashLimiter2.push(12.6);
// stashLimiter2.push(13);
// stashLimiter2.push(13.2);
// stashLimiter2.push(14);
// stashLimiter2.push(15);

// test('Estimated real time', async () => {
//   let stashLimiter1 = new RateLimiter(3, 2);
//   let stashLimiter2 = new RateLimiter(6, 60);

//   let limiters = [stashLimiter1, stashLimiter2];
//   let estimatedTime = limiterWaitInfo(6, limiters);
//   console.log(
//     `Estimated <time=${estimatedTime.estimatedTime / 1000}s,static=${
//       estimatedTime.staticEstimatedTime / 1000
//     }s>`
//   );
//   estimatedTime = limiterWaitInfo(9, limiters); // TODO: <time=0s,static=62s>
//   console.log(
//     `Estimated <time=${estimatedTime.estimatedTime / 1000}s,static=${
//       estimatedTime.staticEstimatedTime / 1000
//     }s>`
//   );
// });

// test('Estimated real time', async () => {
//   let stashLimiter1 = new RateLimiter(3, 2);
//   let stashLimiter2 = new RateLimiter(6, 60);

//   let limiters = [stashLimiter1, stashLimiter2];
//   const stashRequest = 2;
//   const subStashRequests = 2;

//   let estimatedTime = estimateTime(stashRequest, limiters);
//   console.log(`Estimated stash time ${estimatedTime / 1000} s`);

//   estimatedTime = estimateTime(subStashRequests, limiters);
//   console.log(`Estimated substash time ${estimatedTime / 1000} s`);
// });

// test('Current real time', async () => {
//   let stashLimiter1 = new RateLimiter(3, 2);
//   let stashLimiter2 = new RateLimiter(6, 60);

//   let limiters = [stashLimiter1, stashLimiter2];
//   const stashRequest = 2;
//   const subStashRequests = 2;

//   let estimatedTime = estimateTime(stashRequest, limiters);
//   console.log(`Estimated stash time ${estimatedTime / 1000} s`);

//   stashLimiter1.push();
//   stashLimiter1.push();
//   stashLimiter2.push();
//   stashLimiter2.push();

//   estimatedTime = estimateTime(subStashRequests, limiters);
//   console.log(`Estimated substash time ${estimatedTime / 1000} s`);
// });

// test('Estimated future time', async () => {
//   let stashLimiter1 = new RateLimiter(3, 2);
//   let stashLimiter2 = new RateLimiter(6, 60);

//   let limiters = [stashLimiter1, stashLimiter2];
//   const stashRequest = 2;
//   const subStashRequests = 2;

//   let estimatedTime = estimateTime(stashRequest, limiters);
//   console.log(`Estimated stash time ${estimatedTime / 1000} s`);

//   estimatedTime = estimateTime(subStashRequests, limiters, false, false, {
//     requestsToAddBeforeRef: stashRequest,
//     addedRefTime: 0,
//   });
//   console.log(`Estimated substash time ${estimatedTime / 1000} s`);
// });

test('Estimated real time', async () => {
  let stashLimiter1 = new RateLimiter(3, 2);
  let stashLimiter2 = new RateLimiter(6, 60);

  let limiters = [stashLimiter1, stashLimiter2];
  const stashRequest = 2;
  const subStashRequests = 2;

  stashLimiter1.push();

  let estimatedTime = estimateTime(stashRequest, limiters);
  console.log(`Estimated stash time ${estimatedTime / 1000} s`);

  estimatedTime = estimateTime(subStashRequests, limiters);
  console.log(`Estimated substash time ${estimatedTime / 1000} s`);
});
