import { IExternalPrice } from '../interfaces/external-price.interface';
import { IPricedItem } from '../interfaces/priced-item.interface';
import { isUniqueLinkMatch, isVariantMatch, pickBestUniquePrice } from './price-match.utils';

const pricedItem = (overrides: Partial<IPricedItem> = {}): IPricedItem => ({
  uuid: 'priced-item-1',
  name: 'The Ivory Tower',
  itemId: 'item-1',
  typeLine: "Saint's Hauberk",
  frameType: 3,
  total: 0,
  calculated: 0,
  max: 0,
  elder: false,
  shaper: false,
  blighted: false,
  coffin: false,
  beast: false,
  mean: 0,
  median: 0,
  min: 0,
  mode: 0,
  ilvl: 0,
  stackSize: 1,
  totalStacksize: 1,
  links: 0,
  quality: 0,
  level: 0,
  corrupted: false,
  icon: 'item-icon.png',
  sockets: 0,
  variant: '',
  tier: 0,
  inventoryId: 'Stash1',
  tab: [],
  ...overrides,
});

const externalPrice = (overrides: Partial<IExternalPrice> = {}): IExternalPrice => ({
  name: 'The Ivory Tower',
  icon: 'price-icon.png',
  count: 25,
  calculated: 10,
  frameType: 3,
  ...overrides,
});

describe('unique price matching', () => {
  it('treats missing poe.ninja links as unlinked for unique items', () => {
    expect(isUniqueLinkMatch(0, undefined)).toBe(true);
    expect(isUniqueLinkMatch(6, undefined)).toBe(false);
  });

  it('matches singular and plural passive-count variants', () => {
    expect(isVariantMatch('3 Passives', '3 Passive')).toBe(true);
  });

  it('prices unlinked uniques when poe.ninja omits the links field', () => {
    const price = pickBestUniquePrice(pricedItem(), [externalPrice()]);

    expect(price?.calculated).toBe(10);
  });

  it('selects exact variant prices when multiple variant rows exist', () => {
    const price = pickBestUniquePrice(pricedItem({ name: 'Voices', variant: '3 Passives' }), [
      externalPrice({ name: 'Voices', variant: '1 Passive', calculated: 100 }),
      externalPrice({ name: 'Voices', variant: '3 Passive', calculated: 50 }),
    ]);

    expect(price?.calculated).toBe(50);
  });
});
