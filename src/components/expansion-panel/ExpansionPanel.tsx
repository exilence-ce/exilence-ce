import MuiExpansionPanel from '@mui/material/Accordion';
import MuiExpansionPanelDetails from '@mui/material/AccordionDetails';
import MuiExpansionPanelSummary from '@mui/material/AccordionSummary';
import { styled } from '@mui/material/styles';

export const Accordion = styled(MuiExpansionPanel)({
  '&.MuiAccordion-root': {
    border: '1px solid rgba(0, 0, 0, .125)',
    boxShadow: 'none',
    '&:not(:last-child)': {
      borderBottom: 0,
    },
    '&:before': {
      display: 'none',
    },
    '& .Mui-expanded': {
      margin: 'auto',
    },
  },
});

export const AccordionSummary = styled(MuiExpansionPanelSummary)(({ theme }) => ({
  '&.MuiAccordionSummary-root': {
    maxHeight: 40,
    minHeight: '40px !important',
    backgroundColor: theme.palette.secondary.main,
    borderBottom: '1px solid rgba(0, 0, 0, .125)',
    marginBottom: -1,
    '&.Mui-expanded': {
      minHeight: 56,
    },
    padding: '0 16px 0 16px',
  },
  '& .MuiAccordionSummary-content': {
    '&.Mui-expanded': {
      margin: '12px 0',
    },
  },
  '& .MuiTypography-root': {
    lineHeight: 'inherit',
  },
}));

export const AccordionDetails = styled(MuiExpansionPanelDetails)(({ theme }) => ({
  '&.MuiAccordionDetails-root': {
    padding: theme.spacing(2),
  },
}));
