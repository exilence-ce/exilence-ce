import createStyles from '@mui/styles/createStyles';
import makeStyles from '@mui/styles/makeStyles';

import { primaryLighter } from '../../assets/themes/exilence-theme';

export const useStyles = makeStyles((theme) =>
  createStyles({
    poeNinjaCredit: {
      right: theme.spacing(2),
    },
    creditText: {
      color: theme.palette.text.secondary,
    },
    inlineLink: {
      color: primaryLighter,
      verticalAlign: 'baseline',
      textDecoration: 'none',
    },
    secondaryHeader: {
      height: '100%',
      display: 'flex',
      justifyContent: 'flex-end',
      alignSelf: 'flex-end',
    },
  })
);
