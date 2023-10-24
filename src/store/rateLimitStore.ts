/* eslint-disable no-fallthrough */
import { action, autorun, makeObservable, observable } from 'mobx';
import { persist } from 'mobx-persist';
import moment from 'moment';
import { StashLatencyTimer } from '../services/external.service';
import { RateLimiter, ResourceHandle } from './domains/rateLimiter';
import { RootStore } from './rootStore';

const DEBUG = false;

interface IEstimatedTime {
  estimated: number;
  estimatedStatic?: moment.Duration;
}

interface ITimeEstimation {
  measuredTime: number[];
  estimatedTime: number[][];
  measuredLatencys: number[];
  estimatedLatencys: number[][];
  totalEstimatedTime: number[][];
  totalEstimatedLatencys: number[][];
}

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

  timeEstimation?: ITimeEstimation;
  snapshotTimeStart = 0;
  totalEstimatedTime = 0;

  stashLatencyTimer?: StashLatencyTimer;
  @persist('object') snapshotLatencyList = {
    'stash-request-limit': 300,
    'stash-list-request-limit': 300,
    // 'character-request-limit': 600,
    'push-snapshot': 100, // Backend
  };

  @observable @persist retryAfter = 0;
  retryAfterHandle?: ResourceHandle;
  @observable estimatedSnapshotTime: IEstimatedTime = {
    estimated: 0,
    estimatedStatic: undefined,
  };

  constructor(private rootStore: RootStore) {
    makeObservable(this);
    this.initReactionHandler();
  }

  //#region MobX reaction handler
  initReactionHandler() {
    // Automatically ensure the isolated stash tabs if they changed; If necessary make a snapshot to set the baseline
    autorun(() => {
      DEBUG &&
        console.log(
          `Autoreact triggered: setEstimatedSnapshotTime -> <isSnapshotting=${this.rootStore.uiStateStore.isSnapshotting}>`
        );
      // Trigger on isSnapshotting
      this.rootStore.uiStateStore.isSnapshotting;
      // Trigger on profile change
      this.rootStore.accountStore.getSelectedAccount.activeProfile;
      // The action to be triggered
      this.setEstimatedSnapshotTime();
    });
  }
  //#endregion

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
      this.retryAfterHandle.promise.catch(() => {});
    }
  }

  @action
  setEstimatedSnapshotTime() {
    const profile = this.rootStore.accountStore?.getSelectedAccount.activeProfile;
    if (!profile) {
      this.estimatedSnapshotTime = {
        estimated: 0,
        estimatedStatic: undefined,
      };
      return;
    }

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

    let totalTime = 0;
    let totalStaticTime = 0;
    let totalLatency = 0;
    let time = 0;
    let latency = 0;
    let openRequests = 0;
    if (this.rootStore.uiStateStore.isSnapshotting) {
      // Calc the current estimated snapshot time
      switch (this.rootStore.uiStateStore.statusMessage?.message) {
        case 'refreshing_stash_tabs':
          time = RateLimiter.estimateTime(1, this.stashListLimit, true);
          latency = this.snapshotLatencyList['stash-list-request-limit'];
          time -= latency;
          if (time < 0) time = 0;
          totalTime += time;
          totalLatency += latency;

          if (DEBUG) {
            openRequests += 1;
            this.timeEstimation = {
              measuredTime: [],
              estimatedTime: [[openRequests, time]],
              measuredLatencys: [],
              estimatedLatencys: [
                [openRequests, latency, this.snapshotLatencyList['stash-list-request-limit']],
              ],
              totalEstimatedTime: [],
              totalEstimatedLatencys: [],
            };
            this.snapshotTimeStart = moment.utc().valueOf();
          }

        case 'fetching_stash_tab':
          // Fetching character & parent stashtabs - Character and Stashtabs are parallel requested, but at different amount of requests
          if (this.rootStore.uiStateStore.statusMessage?.message === 'fetching_stash_tab') {
            const currentCount = (this.rootStore.uiStateStore.statusMessage.currentCount || 1) - 1;
            expectedStashTabRequests = expectedStashTabRequests - currentCount;
          }
          // We can safely ignore character fetching becasue stash-tabs is always > in time investment
          // ExpectedStashTabRequests shares the same RateLimiter and can cause waittime - but only in combination -> one simulution
          time = RateLimiter.estimateTime(
            expectedStashTabRequests + expectedSubStashTabRequests,
            this.stashLimit,
            true
          );
          latency =
            this.snapshotLatencyList['stash-request-limit'] *
            (expectedStashTabRequests + expectedSubStashTabRequests);
          time -= latency;
          if (time < 0) time = 0;
          totalTime += time;
          totalLatency += latency;

          if (DEBUG) {
            openRequests += expectedStashTabRequests + expectedSubStashTabRequests;
            this.timeEstimation?.estimatedTime.push([
              expectedStashTabRequests + expectedSubStashTabRequests,
              time,
            ]);
            this.timeEstimation?.estimatedLatencys.push([
              expectedStashTabRequests + expectedSubStashTabRequests,
              latency,
              this.snapshotLatencyList['stash-request-limit'],
            ]);
          }
        case 'fetching_subtabs':
          // Fetching substashtabs
          if (this.rootStore.uiStateStore.statusMessage?.message === 'fetching_subtabs') {
            const currentCount = (this.rootStore.uiStateStore.statusMessage.currentCount || 1) - 1;
            expectedSubStashTabRequests = expectedSubStashTabRequests - currentCount;
            // This time is already included in fetching_stash_tab
            time = RateLimiter.estimateTime(expectedSubStashTabRequests, this.stashLimit, true);
            latency = this.snapshotLatencyList['stash-request-limit'] * expectedSubStashTabRequests;
            time -= latency;
            if (time < 0) time = 0;
            totalTime += time;
            totalLatency += latency;

            if (DEBUG) {
              openRequests += expectedSubStashTabRequests;
              this.timeEstimation?.estimatedTime.push([expectedSubStashTabRequests, time]);
              this.timeEstimation?.estimatedLatencys.push([
                expectedSubStashTabRequests,
                latency,
                this.snapshotLatencyList['stash-request-limit'],
              ]);
            }
          }
        case 'pricing_items':
        case 'saving_snapshot': {
          if (DEBUG) {
            this.timeEstimation?.totalEstimatedTime.push([openRequests, totalTime]);
            this.timeEstimation?.totalEstimatedLatencys.push([openRequests, totalLatency]);
          }
          totalLatency += this.snapshotLatencyList['push-snapshot'] * 1;
          totalTime += totalLatency;

          if (DEBUG && this.rootStore.uiStateStore.statusMessage?.message === 'saving_snapshot') {
            const totalSnapshotTime = moment.utc().diff(this.snapshotTimeStart);
            console.log(
              `Total snapshot time was: ${totalSnapshotTime / 1000}s totalEstimation first time: ${
                (this.timeEstimation!.totalEstimatedTime[0][1] +
                  this.timeEstimation!.totalEstimatedLatencys[0][1]) /
                1000
              }s; Realdiff ${Math.abs(
                totalSnapshotTime -
                  (this.timeEstimation!.totalEstimatedTime[0][1] +
                    this.timeEstimation!.totalEstimatedLatencys[0][1])
              )}`
            );

            const totalMeasuredLatency = this.timeEstimation!.measuredLatencys.reduce(
              (prev, val) => prev + val,
              0
            );
            const totalMeasuredTime = this.timeEstimation!.measuredTime.reduce(
              (prev, val) => prev + val,
              0
            );
            const totalLatencyDiff = Math.abs(
              totalMeasuredLatency - this.timeEstimation!.totalEstimatedLatencys[0][1]
            );
            const totalTimeDiff = Math.abs(
              totalMeasuredTime - this.timeEstimation!.totalEstimatedTime[0][1]
            );
            console.log(
              `<totalMeasuredLatency=${totalMeasuredLatency};totalEstimatedLatency(first)=${
                this.timeEstimation!.totalEstimatedLatencys[0][1]
              }> <totalMeasuredTime=${totalMeasuredTime};totalEstimatedTime=${
                this.timeEstimation!.totalEstimatedTime[0][1]
              }> <totalLatencyDiff=${totalLatencyDiff};totalTimeDiff=${totalTimeDiff};totalDiff=${
                totalLatencyDiff + totalTimeDiff
              }>`
            );
            console.log(this.timeEstimation);
          }
          break;
        }
        default:
          break;
      }
    } else {
      const stashListTimes = RateLimiter.limiterWaitInfo(1, this.stashListLimit);
      const characterTimes = RateLimiter.limiterWaitInfo(1, this.characterLimit);
      // We have to combine stash and substash.
      // For example: 15 requests (stacks) + 25 requests (substacks) => 40 requests
      // We have to calc 40 % 30 = 10 requests which are awaitable. After 10 we have another 30 static wait time which is not awaitable. Only within the snapshot itself
      const stashTimes = RateLimiter.limiterWaitInfo(
        expectedStashTabRequests + expectedSubStashTabRequests,
        this.stashLimit
      );
      // Take the greatest estimated time - 300sec is max, if one ratelimiter is full, we have to wait. Take the greatest filled ratelimiter
      totalTime = Math.max(
        stashListTimes.estimatedTime,
        characterTimes.estimatedTime,
        stashTimes.estimatedTime
      );
      // Stashlist and Character does not have static time for 1 request
      totalStaticTime = stashTimes.staticEstimatedTime;
    }
    DEBUG &&
      console.info(
        `EstimatedSnapshotTime ${totalTime / 1000} s <${
          this.rootStore.uiStateStore.statusMessage?.message || ''
        }> ${
          this.rootStore.uiStateStore.isSnapshotting
        } <activeStashTabs=${activeStashTabs},stashRequests=${expectedStashTabRequests},subStashRequest=${expectedSubStashTabRequests}> <latency=${
          totalLatency / 1000
        },${this.snapshotLatencyList['stash-request-limit']}>`
      );

    this.estimatedSnapshotTime = {
      estimated: totalTime <= 0 ? 0 : moment.utc().add(totalTime, 'milliseconds').valueOf(),
      estimatedStatic:
        totalStaticTime <= 0 ? undefined : moment.duration(totalStaticTime, 'milliseconds'),
    };
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
    const DESYNC_FIX = 0.0; // apiLatencySeconds - We can ignore this, since we wait for each request to complete. Max 1 request is in queue

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
