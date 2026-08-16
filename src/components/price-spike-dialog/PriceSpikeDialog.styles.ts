import makeStyles from '@mui/styles/makeStyles';
import { statusColors } from '../../assets/themes/exilence-theme';

const useStyles = makeStyles((theme) => ({
  dialogActions: {
    padding: theme.spacing(2),
    justifyContent: 'space-between',
  },
  warningIcon: {
    color: statusColors.warning,
  },
  itemIcon: {
    width: 32,
    height: 32,
    objectFit: 'contain',
  },
  increase: {
    color: statusColors.warning,
    fontWeight: 'bold',
  },
  itemRow: {
    paddingTop: theme.spacing(1),
    paddingBottom: theme.spacing(1),
  },
  bulkActions: {
    display: 'flex',
    gap: theme.spacing(1),
  },
}));

export default useStyles;
