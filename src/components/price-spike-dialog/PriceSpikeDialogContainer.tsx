import { observer } from 'mobx-react-lite';
import React from 'react';
import { useStores } from '../..';
import { PriceSpikeResolution } from '../../interfaces/price-spike.interface';
import PriceSpikeDialog from './PriceSpikeDialog';

const PriceSpikeDialogContainer = () => {
  const { uiStateStore, priceSpikeStore } = useStores();

  const handleDecision = (uuid: string, resolution: PriceSpikeResolution) => {
    priceSpikeStore.setDecision(uuid, resolution);
  };

  const handleDecisionForAll = (resolution: PriceSpikeResolution) => {
    priceSpikeStore.setDecisionForAllPending(resolution);
  };

  return (
    <>
      {uiStateStore!.priceSpikeDialogOpen && (
        <PriceSpikeDialog
          show={uiStateStore!.priceSpikeDialogOpen}
          spikes={priceSpikeStore.pendingSpikes}
          onClose={() => uiStateStore!.setPriceSpikeDialogOpen(false)}
          onDecision={handleDecision}
          onDecisionForAll={handleDecisionForAll}
        />
      )}
    </>
  );
};

export default observer(PriceSpikeDialogContainer);
