import { useStores } from '../../../index';
import { default as React } from 'react';
import { observer } from 'mobx-react-lite';
import ItemTableLatestSnapshotSelect from './ItemTableLatestSnapshotSelect';
import ItemTableCompareSnapshotsSelect from './ItemTableCompareSnapshotsSelect';

const ItemTableSnapshotSelect = () => {
  const { uiStateStore } = useStores();

  switch (uiStateStore.itemTableSelection) {
    case 'latest':
      return <ItemTableLatestSnapshotSelect />;

    case 'comparison':
      return <ItemTableCompareSnapshotsSelect />;

    default: {
      const state: unknown = uiStateStore.itemTableSelection;
      console.error(`Unknown table selection: ${state}`);
      return <></>;
    }
  }
};

export default observer(ItemTableSnapshotSelect);
