jest.mock('..', () => ({
  rootStore: {},
}));

import { IStashTabSnapshot } from '../interfaces/stash-tab-snapshot.interface';
import { Snapshot } from '../store/domains/snapshot';
import { mapSnapshotToApiSnapshot } from './snapshot.utils';

describe('mapSnapshotToApiSnapshot location mapping', () => {
  it('preserves synthetic character snapshots when no stash tab metadata exists', () => {
    const snapshot = new Snapshot({
      stashTabSnapshots: [
        {
          stashTabId: 'Character',
          value: 42,
          pricedItems: [],
        } as IStashTabSnapshot,
      ],
    });

    const apiSnapshot = mapSnapshotToApiSnapshot(snapshot, []);

    expect(apiSnapshot.stashTabs[0].stashTabId).toBe('Character');
    expect(apiSnapshot.stashTabs[0].name).toBe('Character');
    expect(apiSnapshot.stashTabs[0].value).toBe(42);
  });
});
