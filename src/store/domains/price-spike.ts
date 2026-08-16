import { action, computed, makeObservable, observable } from 'mobx';
import { persist } from 'mobx-persist';
import moment from 'moment';
import { v4 as uuidv4 } from 'uuid';
import { IPriceSpike, PriceSpikeDecision } from '../../interfaces/price-spike.interface';
import { getPriceIncreasePercent } from '../../utils/price.utils';

export class PriceSpike implements IPriceSpike {
  @persist uuid: string = uuidv4();

  @persist leagueId: string = '';
  @persist itemKey: string = '';
  @persist name: string = '';
  @persist icon: string = '';

  /** the last price we consider trustworthy, used when the user keeps the old price */
  @persist @observable previousPrice: number = 0;
  @persist @observable currentPrice: number = 0;
  @persist @observable detectedAt: string = moment().utc().toISOString();
  @persist @observable decision: PriceSpikeDecision = 'pending';

  constructor(obj?: Partial<IPriceSpike>) {
    makeObservable(this);
    Object.assign(this, obj);
  }

  @computed
  get increasePercent() {
    return getPriceIncreasePercent(this.previousPrice, this.currentPrice);
  }

  @action
  setCurrentPrice(price: number) {
    this.currentPrice = price;
  }

  @action
  setDecision(decision: PriceSpikeDecision) {
    this.decision = decision;
  }
}
