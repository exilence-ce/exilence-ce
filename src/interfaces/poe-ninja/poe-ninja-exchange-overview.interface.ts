export interface IPoeNinjaExchangeOverviewLine {
  id: string;
  primaryValue: number;
}

export interface IPoeNinjaExchangeOverviewItem {
  id: string;
  name: string;
  image?: string;
}

export interface IPoeNinjaExchangeOverview {
  lines: IPoeNinjaExchangeOverviewLine[];
  items: IPoeNinjaExchangeOverviewItem[];
}
