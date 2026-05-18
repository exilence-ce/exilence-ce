import { IItem } from '../interfaces/item.interface';
import { getItemVariant, mapItemsToPricedItems } from './item.utils';

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
  typeLine: '',
  identified: true,
  corrupted: false,
  lockedToCharacter: false,
  requirements: [],
  implicitMods: [],
  explicitMods: [],
  fracturedMods: [],
  frameType: 0,
  x: 0,
  y: 0,
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

describe('getItemVariant', () => {
  it('detects Voices passive-count variants from item mods', () => {
    expect(getItemVariant([], ['Adds 3 Passive Skills'], 'Voices')).toBe('3 Passives');
  });

  it("detects Yriel's Fostering beast variants from item mods", () => {
    expect(
      getItemVariant([], ['Grants Level 20 Summon Bestial Rhoa Skill'], "Yriel's Fostering")
    ).toBe('Rhoa');
  });

  it('does not treat every Impresence as the lightning variant', () => {
    expect(getItemVariant([], ['Adds Fire Damage'], 'Impresence')).toBe('Fire');
  });
});

describe('mapItemsToPricedItems', () => {
  it('passes the normalized unique name into variant detection', () => {
    const item = createItem({
      name: 'Voices',
      typeLine: 'Large Cluster Jewel',
      frameType: 3,
      explicitMods: ['Adds 5 Passive Skills'],
    });

    expect(mapItemsToPricedItems([item])[0].variant).toBe('5 Passives');
  });
});
