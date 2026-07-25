import { IPoeNinjaItemOverviewLineSparkline } from './poe-ninja-item-overview-line-spark-line.interface';

export interface IPoeNinjaExchangeOverviewLine {
  id: string;
  primaryValue: number;
  volumePrimaryValue: number;
  maxVolumeCurrency?: string;
  maxVolumeRate?: number;
  sparkline?: IPoeNinjaItemOverviewLineSparkline;
}

export interface IPoeNinjaExchangeOverviewItem {
  id: string;
  name: string;
  image?: string;
  category?: string;
  detailsId?: string;
}

export interface IPoeNinjaExchangeOverviewCore {
  primary: string;
  secondary: string;
  rates: { [currencyId: string]: number };
  items: IPoeNinjaExchangeOverviewItem[];
}

export interface IPoeNinjaExchangeOverview {
  core: IPoeNinjaExchangeOverviewCore;
  lines: IPoeNinjaExchangeOverviewLine[];
  items: IPoeNinjaExchangeOverviewItem[];
}
