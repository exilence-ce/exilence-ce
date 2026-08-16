import { action, computed, makeObservable, observable } from 'mobx';
import { persist } from 'mobx-persist';
import moment from 'moment';
import { IPricedItem } from '../interfaces/priced-item.interface';
import { PriceSpikeResolution } from '../interfaces/price-spike.interface';
import { IStashTabSnapshot } from '../interfaces/stash-tab-snapshot.interface';
import { getItemPriceKey, getPriceIncreasePercent } from '../utils/price.utils';
import { PriceSpike } from './domains/price-spike';
import { Snapshot } from './domains/snapshot';
import { RootStore } from './rootStore';

/**
 * Guards snapshots against manipulated prices.
 *
 * Items whose price explodes between two snapshots (a common symptom of price
 * fixing on the trade site) are held back for the user to review instead of
 * being taken at face value. A held item is excluded from net worth and income
 * until the user either approves the new price or keeps the last trusted one.
 */
export class PriceSpikeStore {
  @persist('list', PriceSpike) @observable spikes: PriceSpike[] = [];

  /** a price increase larger than this between two snapshots is held for review */
  @persist @observable thresholdPercent: number = 1000;
  /**
   * a held item only returns to normal pricing once its increase drops below
   * this. The gap to thresholdPercent is deliberate: releasing at the detection
   * threshold would let an item back into net worth while still priced several
   * times above the value it was trusted at.
   */
  @persist @observable releaseThresholdPercent: number = 500;
  /** spikes are only detected when the previous snapshot is at most this old */
  @persist @observable maxSnapshotAgeDays: number = 7;

  constructor(private rootStore: RootStore) {
    makeObservable(this);
  }

  @computed
  get activeLeagueId() {
    return this.rootStore.accountStore.getSelectedAccount.activeProfile?.activePriceLeagueId ?? '';
  }

  @computed
  get pendingSpikes() {
    return this.spikes.filter(
      (s) => s.decision === 'pending' && s.leagueId === this.activeLeagueId
    );
  }

  @computed
  get pendingSpikeCount() {
    return this.pendingSpikes.length;
  }

  private findSpike(leagueId: string, itemKey: string) {
    return this.spikes.find((s) => s.leagueId === leagueId && s.itemKey === itemKey);
  }

  @action
  setDecision(uuid: string, resolution: PriceSpikeResolution) {
    const spike = this.spikes.find((s) => s.uuid === uuid);
    if (spike) {
      spike.setDecision(resolution);
    }
    this.closeDialogWhenResolved();
  }

  @action
  setDecisionForAllPending(resolution: PriceSpikeResolution) {
    this.pendingSpikes.forEach((s) => s.setDecision(resolution));
    this.closeDialogWhenResolved();
  }

  @action
  closeDialogWhenResolved() {
    if (this.pendingSpikeCount === 0) {
      this.rootStore.uiStateStore.setPriceSpikeDialogOpen(false);
    }
  }

  @action
  removeSpikes(uuids: string[]) {
    this.spikes = this.spikes.filter((s) => !uuids.includes(s.uuid));
  }

  /**
   * Applies the guard to freshly priced stash tabs.
   *
   * Returns the tabs with held items zeroed out (so they drop out of the
   * snapshot and every value derived from it) and kept items re-priced at the
   * last trusted price.
   */
  @action
  applyPriceGuard(
    stashTabs: IStashTabSnapshot[],
    leagueId: string,
    previousSnapshot?: Snapshot
  ): IStashTabSnapshot[] {
    if (!leagueId) {
      return stashTabs;
    }

    const currentPrices = this.getCurrentPrices(stashTabs);
    const previousPrices = this.getPreviousPrices(previousSnapshot);
    const canDetectNewSpikes =
      previousSnapshot !== undefined &&
      moment(previousSnapshot.created)
        .utc()
        .isAfter(moment().utc().subtract(this.maxSnapshotAgeDays, 'days'));

    // key -> price to use, or undefined when the item must be held back
    const overrides: { [key: string]: number | undefined } = {};
    const resolvedSpikes: string[] = [];
    const newSpikes: PriceSpike[] = [];

    currentPrices.forEach((item, key) => {
      const existing = this.findSpike(leagueId, key);

      if (existing) {
        // an approved price is applied once, after which the item is priced
        // normally again and its new price becomes the reference
        if (existing.decision === 'approved') {
          resolvedSpikes.push(existing.uuid);
          return;
        }

        existing.setCurrentPrice(item.calculated);

        // the market came back to a sane level, so the guard is no longer needed
        if (this.canRelease(existing.previousPrice, item.calculated)) {
          resolvedSpikes.push(existing.uuid);
          return;
        }

        // still manipulated: keep honouring the user's choice, or keep holding
        // the item until they make one
        overrides[key] = existing.decision === 'kept' ? existing.previousPrice : undefined;
        return;
      }

      if (!canDetectNewSpikes) {
        return;
      }

      const previousPrice = previousPrices[key];
      if (previousPrice === undefined || !this.exceedsThreshold(previousPrice, item.calculated)) {
        return;
      }

      newSpikes.push(
        new PriceSpike({
          leagueId,
          itemKey: key,
          name: item.name,
          icon: item.icon,
          previousPrice,
          currentPrice: item.calculated,
          detectedAt: moment().utc().toISOString(),
          decision: 'pending',
        })
      );
      overrides[key] = undefined;
    });

    // pending reviews for items that left the stash are no longer actionable
    const stalePending = this.spikes
      .filter(
        (s) => s.leagueId === leagueId && s.decision === 'pending' && !currentPrices.has(s.itemKey)
      )
      .map((s) => s.uuid);

    this.removeSpikes([...resolvedSpikes, ...stalePending]);
    this.spikes = this.spikes.concat(newSpikes);

    if (newSpikes.length > 0) {
      this.rootStore.uiStateStore.setPriceSpikeDialogOpen(true);
      this.rootStore.notificationStore.createNotification('price_spike_detected', 'warning', true);
    }

    return this.applyOverrides(stashTabs, overrides);
  }

  private exceedsThreshold(previousPrice: number, currentPrice: number) {
    if (previousPrice <= 0) {
      return false;
    }
    return getPriceIncreasePercent(previousPrice, currentPrice) > this.thresholdPercent;
  }

  private canRelease(previousPrice: number, currentPrice: number) {
    if (previousPrice <= 0) {
      return true;
    }
    return getPriceIncreasePercent(previousPrice, currentPrice) < this.releaseThresholdPercent;
  }

  private applyOverrides(
    stashTabs: IStashTabSnapshot[],
    overrides: { [key: string]: number | undefined }
  ) {
    return stashTabs.map((tab) => {
      tab.pricedItems = tab.pricedItems.map((item) => {
        const key = getItemPriceKey(item);
        if (!(key in overrides)) {
          return item;
        }
        // a held item is priced at zero so it is filtered out of the snapshot
        // entirely, keeping it out of net worth and income calculations
        const price = overrides[key] ?? 0;
        return { ...item, calculated: price, total: price * item.stackSize } as IPricedItem;
      });
      return tab;
    });
  }

  private getCurrentPrices(stashTabs: IStashTabSnapshot[]) {
    const prices = new Map<string, IPricedItem>();
    stashTabs.forEach((tab) => {
      tab.pricedItems.forEach((item) => {
        if (item.calculated > 0) {
          const key = getItemPriceKey(item);
          if (!prices.has(key)) {
            prices.set(key, item);
          }
        }
      });
    });
    return prices;
  }

  private getPreviousPrices(previousSnapshot?: Snapshot) {
    const prices: { [key: string]: number } = {};
    previousSnapshot?.stashTabSnapshots.forEach((tab) => {
      tab.pricedItems.forEach((item) => {
        const key = getItemPriceKey(item);
        if (prices[key] === undefined && item.calculated > 0) {
          prices[key] = item.calculated;
        }
      });
    });
    return prices;
  }
}
