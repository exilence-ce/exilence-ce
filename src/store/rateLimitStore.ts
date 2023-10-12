import { action, makeObservable, observable } from 'mobx';
import { persist } from 'mobx-persist';
import moment from 'moment';
import { RateLimiter } from './domains/rateLimiter';
import { RootStore } from './rootStore';

export class RateLimitStore {
  @observable @persist('list', RateLimiter) stashLimit = [
    new RateLimiter('stash-request-limit', 1, 5),
  ];
  @observable @persist('list', RateLimiter) stashListLimit = [
    new RateLimiter('stash-list-request-limit', 1, 5),
  ];
  @observable @persist('list', RateLimiter) characterLimit = [
    new RateLimiter('character-request-limit', 1, 5),
  ];
  // Not involved in snapshot
  @observable @persist('list', RateLimiter) characterListLimit = [
    new RateLimiter('character-list-request-limit', 1, 5),
  ];
  @observable @persist('list', RateLimiter) ladderViewLimit = [
    new RateLimiter('ladder-view', 1, 5),
  ];

  @observable @persist retryAfter = 0;

  constructor(private rootStore: RootStore) {
    makeObservable(this);
  }

  get limiters() {
    return [
      this.stashLimit,
      this.stashListLimit,
      this.characterLimit,
      this.characterListLimit,
      this.ladderViewLimit,
    ];
  }

  get snapshotLimiters() {
    return [this.stashLimit, this.stashListLimit, this.characterLimit];
  }

  // @computed
  // get retrySnapshotAfter() {
  //   const now = moment();
  //   const maxRetryAfter = this.snapshotLimiters
  //     .filter((limiter) => limiter.retryAfter !== 0)
  //     .map((limiter) => moment(limiter.retryAfter).diff(now));
  //   return Math.max(0, ...maxRetryAfter);
  // }

  @action
  setRetryAfter(seconds: number) {
    this.retryAfter = seconds === 0 ? 0 : moment().add(seconds, 'seconds').valueOf();
  }

  @action
  getEstimatedSnapshotTime(stashTabsToRequests: number) {
    // TODO: Calculate 1 request in advance - and stashTabsToRequests in advance
    const stashListTime = RateLimiter.estimateTime(1, this.stashListLimit);
    const characterTime = RateLimiter.estimateTime(1, this.characterLimit);
    let stashTime = 0;
    if (stashTabsToRequests > 0) {
      stashTime = RateLimiter.estimateTime(stashTabsToRequests, this.stashLimit);
    }
    return stashListTime + characterTime + stashTime;
  }

  static async waitMulti(limiters: Iterable<RateLimiter>): Promise<void> {
    const _limiters = Array.from(limiters);

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

  static adjustRateLimits(clientLimits: RateLimiter[], headers: Headers): void {
    if (!headers['x-rate-limit-rules']) return;

    const rules = headers['x-rate-limit-rules']!.toLowerCase().split(',');
    const retryAfter = headers['retry-after'];

    this._adjustRateLimits(
      clientLimits,
      rules.map((rule) => headers[`x-rate-limit-${rule}`]!).join(','),
      rules.map((rule) => headers[`x-rate-limit-${rule}-state`]!).join(','),
      retryAfter
        ? moment()
            .add(+retryAfter, 'seconds')
            .valueOf()
        : 0
    );
  }

  private static _adjustRateLimits(
    clientLimits: RateLimiter[],
    limitStr: string,
    stateStr: string,
    retryAfter: number
  ): void {
    /* eslint-disable no-console */
    const DEBUG = true;
    const DESYNC_FIX = 0.05; // apiLatencySeconds

    const limitRuleState = stateStr
      .split(',')
      .map((rule) => rule.split(':'))
      .map((rule) => Number(rule[0]));
    const limitRule = limitStr
      .split(',')
      .map((rule) => rule.split(':'))
      .map((rule, idx) => ({
        max: Number(rule[0]),
        window: Number(rule[1]) + DESYNC_FIX,
        state: limitRuleState[idx],
      }));

    // destroy
    const policy = clientLimits[0]?.policy;
    for (const limit of clientLimits) {
      const isActive = limitRule.some((serverLimit) => limit.isEqualLimit(serverLimit));
      if (!isActive) {
        clientLimits.splice(clientLimits.indexOf(limit), 1);
        limit.destroy();
        DEBUG && console.log('Destroy', limit.toString());
      }
    }

    // compare client<>server state
    for (const limit of clientLimits) {
      const serverLimit = limitRule.find((serverLimit) => limit.isEqualLimit(serverLimit))!;
      const delta = serverLimit.state - limit.stack.length;

      if (delta === 0) {
        DEBUG && console.log(`<${policy}> Limits are in sync`);
      } else if (delta > 0) {
        DEBUG &&
          console.error(
            `<${policy}> Rate limit state on Server is greater by ${Math.abs(
              delta
            )}. Bursting to prevent rate limiting.`
          );
        for (let i = 0; i < Math.min(delta, limit.available); ++i) {
          // eslint-disable-next-line @typescript-eslint/no-floating-promises
          limit.wait();
        }
      } else if (delta < 0) {
        DEBUG && console.warn(`Rate limit state on Client is greater by ${Math.abs(delta)}`);
      }
    }

    // add new
    serverLimits: for (const serverLimit of limitRule) {
      for (const limit of clientLimits) {
        if (limit.isEqualLimit(serverLimit)) continue serverLimits;
      }

      const rl = new RateLimiter(policy, serverLimit.max, serverLimit.window);
      clientLimits.push(rl);
      DEBUG && console.log('Add', rl.toString());

      if (retryAfter) {
        rl.applyRetryAfter(retryAfter);
      } else {
        for (let i = 0; i < serverLimit.state; ++i) {
          // eslint-disable-next-line @typescript-eslint/no-floating-promises
          rl.wait();
        }
      }
    }
  }

  static preventQueueCreation(targets: Array<{ count: number; limiters: Iterable<RateLimiter> }>) {
    const estimatedMillis = Math.max(
      ...targets.map((target) => {
        const estimated = RateLimiter.estimateTime(target.count, target.limiters);
        const estimatedCleanState = RateLimiter.estimateTime(target.count, target.limiters, true);

        // ignore if impossible to run without queue
        return estimated === estimatedCleanState ? 0 : estimated;
      })
    );

    if (estimatedMillis >= 1500) {
      throw new Error(`Retry after ${Math.round(estimatedMillis / 1000)} seconds`);
    }
  }
}
