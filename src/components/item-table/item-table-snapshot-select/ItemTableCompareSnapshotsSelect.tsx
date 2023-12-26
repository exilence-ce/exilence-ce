import { default as React, useCallback, useMemo } from 'react';
import { observer } from 'mobx-react-lite';
import { FormControl, InputLabel, MenuItem, Select, SelectChangeEvent } from '@mui/material';
import moment from 'moment/moment';
import { useTranslation } from 'react-i18next';
import useStyles from './ItemTableSnapshotSelect.styles';
import { useStores } from '../../../index';
import { findSnapshot } from '../../../utils/snapshot.utils';
import { BaseSnapshotID, HeadSnapshotID } from '../../../store/uiStateStore';

const ItemTableCompareSnapshotsSelect = () => {
  const { t } = useTranslation();
  const classes = useStyles();
  const { uiStateStore, accountStore } = useStores();

  const activeProfile = accountStore!.getSelectedAccount.activeProfile;

  if (!activeProfile) {
    return <></>;
  }

  const snapshots = activeProfile.snapshots || [];
  const baseSnapshot = useMemo(
    () => findSnapshot(snapshots, uiStateStore.itemTableSnapshotComparisonBase),
    [uiStateStore.itemTableSnapshotComparisonBase, snapshots]
  );
  const headSnapshot = useMemo(
    () => findSnapshot(snapshots, uiStateStore.itemTableSnapshotComparisonHead),
    [uiStateStore.itemTableSnapshotComparisonHead, snapshots]
  );

  const handleSnapshotHeadChange = (event: SelectChangeEvent) => {
    uiStateStore.setItemTableSnapshotComparisonHead(event.target.value as HeadSnapshotID);
  };
  const handleSnapshotBaseChange = useCallback(
    (event: SelectChangeEvent) => {
      const baseId = event.target.value as BaseSnapshotID;
      const newBaseSnapshot = findSnapshot(snapshots, baseId);
      if (
        newBaseSnapshot &&
        headSnapshot &&
        moment(newBaseSnapshot.created).isAfter(headSnapshot.created)
      ) {
        uiStateStore.setItemTableSnapshotComparisonHead('latest');
      }

      uiStateStore.setItemTableSnapshotComparisonBase(baseId);
    },
    [headSnapshot, snapshots]
  );

  const filteredSnapshots = useMemo(
    () =>
      snapshots.filter((s) =>
        baseSnapshot ? moment(s.created).isAfter(baseSnapshot.created) : false
      ),
    [baseSnapshot, snapshots]
  );

  return (
    <>
      <FormControl>
        <InputLabel id="select-snapshot-from-label">{t('label.snapshot_from')}</InputLabel>
        <Select
          label={t('label.snapshot_from')}
          labelId="select-snapshot-from-label"
          className={classes.selectionGroup}
          value={uiStateStore.itemTableSnapshotComparisonBase}
          onChange={handleSnapshotBaseChange}
          MenuProps={{ classes: { paper: classes.MenuPaper } }}
        >
          <MenuItem key="second-to-last" value="second-to-last">
            {t('option.second-to-last')}
          </MenuItem>
          {snapshots.map((snapshot) => {
            return (
              <MenuItem key={snapshot.uuid} value={snapshot.uuid}>
                {moment(snapshot.created).format('MMM DD, YYYY HH:mm')}
              </MenuItem>
            );
          })}
        </Select>
      </FormControl>
      <FormControl>
        <InputLabel id="select-snapshot-to-label">{t('label.snapshot_to')}</InputLabel>
        <Select
          label={t('label.snapshot_to')}
          labelId="select-snapshot-to-label"
          className={classes.selectionGroup}
          value={uiStateStore.itemTableSnapshotComparisonHead}
          onChange={handleSnapshotHeadChange}
          MenuProps={{ classes: { paper: classes.MenuPaper } }}
        >
          <MenuItem key="latest" value="latest">
            {t('option.latest')}
          </MenuItem>
          {filteredSnapshots.map((snapshot) => {
            return (
              <MenuItem key={snapshot.uuid} value={snapshot.uuid}>
                {moment(snapshot.created).format('MMM DD, YYYY HH:mm')}
              </MenuItem>
            );
          })}
        </Select>
      </FormControl>
    </>
  );
};

export default observer(ItemTableCompareSnapshotsSelect);
