import WarningIcon from '@mui/icons-material/Warning';
import {
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  Divider,
  Grid,
  Typography,
} from '@mui/material';
import { observer } from 'mobx-react-lite';
import React from 'react';
import { useTranslation } from 'react-i18next';
import { PriceSpikeResolution } from '../../interfaces/price-spike.interface';
import { PriceSpike } from '../../store/domains/price-spike';
import { formatValue } from '../../utils/snapshot.utils';
import useStyles from './PriceSpikeDialog.styles';

type PriceSpikeDialogProps = {
  show: boolean;
  spikes: PriceSpike[];
  onClose: () => void;
  onDecision: (uuid: string, resolution: PriceSpikeResolution) => void;
  onDecisionForAll: (resolution: PriceSpikeResolution) => void;
};

const PriceSpikeDialog = ({
  show,
  spikes,
  onClose,
  onDecision,
  onDecisionForAll,
}: PriceSpikeDialogProps) => {
  const classes = useStyles();
  const { t } = useTranslation();

  return (
    <Dialog open={show} onClose={onClose} maxWidth="md" fullWidth>
      <DialogTitle>
        <Box sx={{ display: 'flex', alignItems: 'center', gap: 1 }}>
          <WarningIcon className={classes.warningIcon} />
          {t('title.price_spike_dialog_title')}
        </Box>
      </DialogTitle>
      <DialogContent>
        <DialogContentText>{t('body.price_spike')}</DialogContentText>
        <Divider />
        {spikes.map((spike) => (
          <Box key={spike.uuid}>
            <Grid container alignItems="center" spacing={2} className={classes.itemRow}>
              <Grid item>
                {spike.icon && (
                  <img src={spike.icon} alt={spike.name} className={classes.itemIcon} />
                )}
              </Grid>
              <Grid item xs>
                <Typography variant="body1">{spike.name}</Typography>
                <Typography variant="body2" color="textSecondary">
                  {t('label.price_spike_change', {
                    oldPrice: formatValue(spike.previousPrice, t('unit.chaos'), false, true),
                    newPrice: formatValue(spike.currentPrice, t('unit.chaos'), false, true),
                  })}{' '}
                  <span className={classes.increase}>
                    ({formatValue(spike.increasePercent, t('unit.percent'), true, true)})
                  </span>
                </Typography>
              </Grid>
              <Grid item>
                <Button
                  size="small"
                  variant="outlined"
                  color="primary"
                  onClick={() => onDecision(spike.uuid, 'kept')}
                >
                  {t('action.keep_old_price')}
                </Button>
              </Grid>
              <Grid item>
                <Button
                  size="small"
                  variant="contained"
                  color="primary"
                  onClick={() => onDecision(spike.uuid, 'approved')}
                >
                  {t('action.approve_new_price')}
                </Button>
              </Grid>
            </Grid>
            <Divider />
          </Box>
        ))}
      </DialogContent>
      <DialogActions className={classes.dialogActions}>
        <Button onClick={onClose}>{t('action.decide_later')}</Button>
        <Box className={classes.bulkActions}>
          <Button variant="outlined" color="primary" onClick={() => onDecisionForAll('kept')}>
            {t('action.keep_all_old_prices')}
          </Button>
          <Button variant="contained" color="primary" onClick={() => onDecisionForAll('approved')}>
            {t('action.approve_all_new_prices')}
          </Button>
        </Box>
      </DialogActions>
    </Dialog>
  );
};

export default observer(PriceSpikeDialog);
