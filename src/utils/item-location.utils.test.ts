import { IItem } from '../interfaces/item.interface';
import { IStashTab } from '../interfaces/stash.interface';
import { mapItemsToPricedItems } from './item.utils';

const createItem = (overrides: Partial<IItem>): IItem => ({
  id: 'item-1',
  verified: true,
  w: 1,
  h: 1,
  ilvl: 1,
  icon: 'icon.png',
  league: 'Standard',
  sockets: [],
  name: '',
  shaper: false,
  elder: false,
  baseType: '',
  fractured: false,
  synthesised: false,
  typeLine: 'Chaos Orb',
  identified: true,
  corrupted: false,
  lockedToCharacter: false,
  requirements: [],
  implicitMods: [],
  explicitMods: [],
  fracturedMods: [],
  frameType: 5,
  x: 3,
  y: 7,
  inventoryId: 'Stash1',
  socketedItems: [],
  properties: [],
  flavourText: [],
  craftedMods: [],
  enchantMods: [],
  utilityMods: [],
  descrText: '',
  prophecyText: '',
  socket: 0,
  ...overrides,
});

const stashTab: IStashTab = {
  id: 'stash-tab-1',
  index: 2,
  name: 'Dump',
  type: 'NormalStash',
  metadata: { colour: '#123456' },
};

describe('mapItemsToPricedItems location fields', () => {
  it('preserves stash tab and grid location data for exports', () => {
    const item = mapItemsToPricedItems([createItem({ id: 'currency-1' })], stashTab)[0];

    expect(item.itemId).toBe('currency-1');
    expect(item.x).toBe(3);
    expect(item.y).toBe(7);
    expect(item.tabId).toBe('stash-tab-1');
    expect(item.tabName).toBe('Dump');
    expect(item.tabIndex).toBe(2);
    expect(item.tabColor).toBe('#123456');
  });

  it('marks character inventory and equipment sources without requiring a stash tab', () => {
    const item = mapItemsToPricedItems(
      [createItem({ inventoryId: 'BodyArmour', x: 0, y: 0 })],
      undefined,
      'Character equipment'
    )[0];

    expect(item.source).toBe('Character equipment');
    expect(item.inventoryId).toBe('BodyArmour');
    expect(item.x).toBe(0);
    expect(item.y).toBe(0);
  });
});
