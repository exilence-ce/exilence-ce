import { Column } from 'react-table';
import { CurrencyHeader } from '../../store/settingStore';
import {
  itemCorrupted,
  itemIcon,
  itemIlvlTier,
  itemLinks,
  itemName,
  itemQuantity,
  itemValue,
  sparkLine,
} from '../columns/Columns';

const itemTableComparisonColumns = (currencyHeaders: CurrencyHeader[]): Column<object>[] => [
  itemIcon({
    accessor: 'icon',
    header: 'Icon',
  }),
  itemName({
    accessor: 'name',
    header: 'Name',
  }),
  itemIlvlTier({
    accessor: (row: any) => (row.tier > 0 ? row.tier : row.ilvl),
    header: 'Ilvl / Tier',
  }),
  itemCorrupted({
    accessor: 'corrupted',
    header: 'Corrupted',
  }),
  itemLinks({
    accessor: 'links',
    header: 'Links',
  }),
  {
    Header: 'Quality',
    accessor: 'quality',
    align: 'right',
    maxWidth: 60,
  },
  {
    Header: 'Level',
    accessor: 'level',
    align: 'right',
    maxWidth: 60,
  },
  itemQuantity({
    header: 'Quantity diff',
    accessor: 'stackSize',
    diff: true,
  }),
  sparkLine({
    accessor: 'sparkLine.totalChange',
    header: 'Price last 7 days',
  }),
  itemValue({
    currencySwitchId: 'calculated',
    accessor: 'calculated',
    header: 'Price',
    currencyHeaders,
  }),
  itemValue({
    currencySwitchId: 'total',
    accessor: 'total',
    header: 'Total value diff',
    diff: true,
    currencyHeaders,
  }),
];

export default itemTableComparisonColumns;
