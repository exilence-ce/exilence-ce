import { default as React } from 'react';
import { observer } from 'mobx-react-lite';
import { FormControl, InputLabel, MenuItem, Select, SelectChangeEvent } from '@mui/material';
import moment from 'moment/moment';
import { useTranslation } from 'react-i18next';
import useStyles from './ItemTableSnapshotSelect.styles';
import { useStores } from '../../../index';
import { HeadSnapshotID } from '../../../store/uiStateStore';

const ItemTableLatestSnapshotSelect = () => {
  const { t } = useTranslation();
  const classes = useStyles();
  const { uiStateStore, accountStore } = useStores();

  const activeProfile = accountStore!.getSelectedAccount.activeProfile;

  if (!activeProfile) {
    return <></>;
  }

  const snapshots = activeProfile.snapshots || [];

  const handleSnapshotChange = (event: SelectChangeEvent) => {
    uiStateStore.setItemTableSnapshotHead(event.target.value as HeadSnapshotID);
  };

  return (
    <FormControl>
      <InputLabel id="select-snapshot-label">{t('label.snapshot')}</InputLabel>
      <Select
        label={t('label.snapshot')}
        labelId="select-snapshot-label"
        className={classes.selectionGroup}
        value={uiStateStore.itemTableSnapshotHead}
        onChange={handleSnapshotChange}
        MenuProps={{ classes: { paper: classes.MenuPaper } }}
      >
        <MenuItem key="latest" value="latest">
          {t('option.latest')}
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
  );
};

export default observer(ItemTableLatestSnapshotSelect);
