/* eslint-disable no-fallthrough */
import { action, computed, makeObservable, observable } from 'mobx';
import { persist } from 'mobx-persist';
import moment from 'moment';
import { RateLimiter, ResourceHandle } from './domains/rateLimiter';
import { RootStore } from './rootStore';

const DEBUG = true;

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
  retryAfterHandle: ResourceHandle | undefined;

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

  @action
  setRetryAfter(seconds: number) {
    this.retryAfter = seconds === 0 ? 0 : moment.utc().add(seconds, 'seconds').valueOf();
    if (seconds > 0) {
      this.retryAfterHandle?.cancel();
      this.retryAfterHandle = new ResourceHandle(seconds * 1000, () => {
        this.retryAfterHandle = undefined;
        this.setRetryAfter(0);
      });
    }
  }

  @computed
  get estimatedSnapshotTime() {
    const profile = this.rootStore.accountStore.getSelectedAccount.activeProfile;
    if (!profile) return 0;

    const latency = 320;
    const activeStashTabs = profile.activeStashTabIds.length;
    let expectedStashTabRequests = activeStashTabs;
    if (
      profile.lastFoundActiveStashTabIds !== undefined &&
      profile.lastFoundActiveStashTabIds < activeStashTabs
    ) {
      expectedStashTabRequests = profile.lastFoundActiveStashTabIds;
    }
    let expectedSubStashTabRequests = 0;
    if (profile.lastFoundActiveSubStashTabIds !== undefined) {
      expectedSubStashTabRequests = profile.lastFoundActiveSubStashTabIds;
    }

    DEBUG && console.log(`Calc snapshot time: Snapshotting: ${this.rootStore.uiStateStore.isSnapshotting} <activeStashTabs=${activeStashTabs},lastParent=${expectedStashTabRequests},lastSub=${expectedSubStashTabRequests}>`;);

    let total = 0;
    if (this.rootStore.uiStateStore.isSnapshotting) {
      // Calc the current estimated snapshot time
      switch (this.rootStore.uiStateStore.statusMessage?.message) {
        case 'refreshing_stash_tabs':
          total = RateLimiter.estimateTime(1, this.stashListLimit);
          total += latency;
        case 'fetching_stash_tab':
          // Fetching character & parent stashtabs
          total += RateLimiter.estimateTime(1, this.characterLimit);
          total += RateLimiter.estimateTime(expectedStashTabRequests, this.stashLimit);
          total += latency * (1 + expectedStashTabRequests);
        case 'fetching_subtabs':
          // Fetching substashtabs
          total += RateLimiter.estimateTime(expectedSubStashTabRequests, this.stashLimit);
          total += latency * expectedSubStashTabRequests;
        case 'pricing_items':
        case 'saving_snapshot':
          total += latency * 0.5; // Backend
          break;
        default:
          break;
      }
    } else {
      // Calc when snapshot is ready without waittime
      total = RateLimiter.estimateTime(1, this.stashListLimit);
      total += RateLimiter.estimateTime(1, this.characterLimit);
      total += RateLimiter.estimateTime(expectedStashTabRequests, this.stashLimit);
      total += RateLimiter.estimateTime(expectedSubStashTabRequests, this.stashLimit);
    }
    DEBUG && console.info(`EstimatedSnapshotTime ${total / 1000} s`);
    if (total === 0) return 0;
    return moment.utc().add(total, 'milliseconds').valueOf();
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
        ? moment
            .utc()
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
        DEBUG && console.log(`Limits are in sync ` + limit.toString());
      } else if (delta > 0) {
        DEBUG &&
          console.error(
            `Rate limit state on Server is greater by ${Math.abs(
              delta
            )}. Bursting to prevent rate limiting. ` + limit.toString()
          );
        for (let i = 0; i < Math.min(delta, limit.available); ++i) {
          // eslint-disable-next-line @typescript-eslint/no-floating-promises
          limit.wait();
        }
      } else if (delta < 0) {
        DEBUG &&
          console.warn(
            `Rate limit state on Client is greater by ${Math.abs(delta)} ` + limit.toString()
          );
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
