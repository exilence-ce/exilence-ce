/** the choice a user can make about a held item */
export type PriceSpikeResolution = 'approved' | 'kept';

export type PriceSpikeDecision = 'pending' | PriceSpikeResolution;

export interface IPriceSpike {
  uuid: string;
  leagueId: string;
  itemKey: string;
  name: string;
  icon: string;
  previousPrice: number;
  currentPrice: number;
  detectedAt: string;
  decision: PriceSpikeDecision;
}
