import { rootStore } from '..';
import { IExternalPrice } from '../interfaces/external-price.interface';
import { IPoeNinjaCurrencyOverviewCurrencyDetail } from '../interfaces/poe-ninja/poe-ninja-currency-overview-currency-detail.interface';
import { IPoeNinjaCurrencyOverviewLine } from '../interfaces/poe-ninja/poe-ninja-currency-overview-line.interface';
import {
  IPoeNinjaExchangeOverviewItem,
  IPoeNinjaExchangeOverviewLine,
} from '../interfaces/poe-ninja/poe-ninja-exchange-overview.interface';
import { IPoeNinjaItemOverviewLine } from '../interfaces/poe-ninja/poe-ninja-item-overview-line.interface';
import { IPoeWatchCombinedPriceItemData } from '../interfaces/poe-watch/poe-watch-combined-price-item-data.interface';
import { IPricedItem } from '../interfaces/priced-item.interface';
import { ISparklineDataPoint } from '../interfaces/sparkline-data-point.interface';
import AppConfig from './../config/app.config';
import { getNinjaLeagueUrl, getNinjaTypeUrl } from './ninja.utils';

export function getExternalPriceFromWatchItem(
  item: IPoeWatchCombinedPriceItemData
): IExternalPrice {
  return {
    name: item.name,
    icon: item.icon,
    max: item.max ?? 0,
    mean: item.mean ?? 0,
    median: item.median ?? 0,
    min: item.min ?? 0,
    mode: item.mode ?? 0,
    frameType: item.frame ?? 0,
    shaper: item.baseIsShaper,
    elder: item.baseIsElder,
    links: item.linkCount ?? 0,
    level: item.gemLevel ?? 0,
    ilvl: item.baseItemLevel ?? 0,
    variant: item.variation,
    baseType: item.type,
    corrupted: item.gemIsCorrupted,
    calculated: 0,
    totalStacksize: item.stackSize ?? 0,
    tier: item.tier ?? 0,
    quality: item.gemQuality,
  } as IExternalPrice;
}

export function getExternalPriceFromNinjaItem(
  item: IPoeNinjaItemOverviewLine,
  type: string,
  league: string
) {
  const detailsUrl = `${AppConfig.poeNinjaBaseUrl}/poe1/economy/${getNinjaLeagueUrl(
    league
  )}/${getNinjaTypeUrl(type)}/${item.detailsId}`;
  // the current api no longer sends mapTier; the tier is embedded in the name,
  // e.g "Map (Tier 16)", which matches the ingame baseType for maps
  const mapTierMatch = / \(Tier (\d+)\)$/.exec(item.name);
  const sparkLine = item.sparkLine ?? item.sparkline;
  return {
    name: item.name,
    icon: item.icon,
    calculated: item.chaosValue ?? 0,
    links: item.links ?? 0,
    variant: item.variant !== null && item.variant !== undefined ? item.variant : '',
    elder: item.variant === 'Elder' ? true : false,
    shaper: item.variant === 'Shaper' ? true : false,
    level: item.gemLevel ?? 0,
    frameType: item.itemClass ?? 0,
    baseType: item.baseType,
    ilvl: item.levelRequired ?? 0,
    corrupted: item.corrupted ?? false,
    totalStacksize: item.stackSize ?? 0,
    tier: item.mapTier ?? (mapTierMatch ? +mapTierMatch[1] : 0),
    count: item.count ?? 0,
    quality: item.gemQuality ?? 0,
    detailsUrl: detailsUrl,
    sparkLine: item.count > 10 ? sparkLine : item.lowConfidenceSparkline ?? sparkLine,
  } as IExternalPrice;
}

const poeCdnBaseUrl = 'https://web.poecdn.com';
// every divination card shares the same inventory art; the exchange overview carries
// no per-card image, and the pricing service identifies cards by 'Inventory' in the icon
const divinationCardIcon = `${poeCdnBaseUrl}/gen/image/WzI1LDE0LHsiZiI6IjJESXRlbXMvRGl2aW5hdGlvbi9JbnZlbnRvcnlJY29uIiwidyI6MSwiaCI6MSwic2NhbGUiOjF9XQ/f34bf8cbb5/InventoryIcon.png`;

export function getExternalPriceFromNinjaExchangeLine(
  line: IPoeNinjaExchangeOverviewLine,
  item: IPoeNinjaExchangeOverviewItem | undefined,
  type: string,
  league: string
) {
  const detailsUrl = `${AppConfig.poeNinjaBaseUrl}/poe1/economy/${getNinjaLeagueUrl(
    league
  )}/${getNinjaTypeUrl(type)}/${item?.detailsId ?? line.id}`;
  const calculated = line.primaryValue ?? 0;
  let icon = item?.image;
  if (icon && icon.indexOf('http') !== 0) {
    icon = `${poeCdnBaseUrl}${icon}`;
  }
  if (!icon && type === 'DivinationCard') {
    icon = divinationCardIcon;
  }
  return {
    name: item?.name ?? line.id,
    icon: icon,
    calculated: calculated,
    links: 0,
    variant: '',
    elder: false,
    shaper: false,
    level: 0,
    frameType: 5,
    ilvl: 0,
    corrupted: false,
    totalStacksize: 0,
    tier: 0,
    // the exchange overview has no observation count; approximate one from traded
    // volume, but never below the low-confidence threshold since these prices come
    // from completed ingame currency-exchange trades rather than listings
    count:
      calculated > 0 ? Math.max(11, Math.round((line.volumePrimaryValue ?? 0) / calculated)) : 0,
    quality: 0,
    detailsUrl: detailsUrl,
    sparkLine: line.sparkline,
  } as IExternalPrice;
}

export function formatSparklineChartData(data: number[]): ISparklineDataPoint[] | undefined {
  if (data.length === 0) {
    return;
  }
  return data.map((val, i) => {
    return {
      x: i + 1,
      y: val,
    } as ISparklineDataPoint;
  });
}

export function getExternalPriceFromNinjaCurrencyItem(
  item: IPoeNinjaCurrencyOverviewLine,
  details: IPoeNinjaCurrencyOverviewCurrencyDetail | undefined,
  type: string,
  league: string
) {
  const detailsUrl = `${AppConfig.poeNinjaBaseUrl}/poe1/economy/${getNinjaLeagueUrl(
    league
  )}/${getNinjaTypeUrl(type)}/${item.detailsId}`;
  const calculated = item.receive ? item.receive.value : 0;
  const sparkLine = item.receiveSparkLine ? item.receiveSparkLine : undefined;

  return {
    name: item.currencyTypeName,
    calculated: calculated,
    icon: details !== undefined ? details.icon : undefined,
    count: item.receive ? item.receive.count : 0,
    frameType: 5,
    detailsUrl: detailsUrl,
    sparkLine:
      item.receive && item.receive.count > 10 ? sparkLine : item.lowConfidenceReceiveSparkLine,
  } as IExternalPrice;
}

// the same item can appear in several overviews (e.g Divine Orb in both the stash
// currency overview and the exchange overview); keep the occurrence backed by the
// most observations so it survives the low-confidence filter
export function dedupePrices(prices: IExternalPrice[]) {
  const indexByKey: { [key: string]: number } = {};
  const deduped: IExternalPrice[] = [];
  prices.forEach((p) => {
    const key = [
      p.name,
      p.frameType ?? 0,
      p.quality ?? 0,
      p.links ?? 0,
      p.level ?? 0,
      p.corrupted ?? false,
      p.variant ?? '',
      p.tier ?? 0,
      p.ilvl ?? 0,
    ].join('|');
    const existingIndex = indexByKey[key];
    if (existingIndex === undefined) {
      indexByKey[key] = deduped.length;
      deduped.push(p);
    } else if ((p.count ?? 0) > (deduped[existingIndex].count ?? 0)) {
      deduped[existingIndex] = p;
    }
  });
  return deduped;
}

export const filterPrices = (prices: IExternalPrice[]) => {
  if (prices.length === 0) {
    return [];
  }
  const filterText = rootStore.uiStateStore.priceTableFilterText.toLowerCase();

  return prices.filter((p) => {
    return p.name.toLowerCase().includes(filterText);
  });
};

export function mapPriceToItem(item: IPricedItem, price: IExternalPrice, customPrice?: number) {
  if (price !== undefined) {
    item.calculated = customPrice ? customPrice : price.calculated || 0;
    item.max = price.max || 0;
    item.mean = price.mean || 0;
    item.mode = price.mode || 0;
    item.min = price.min || 0;
    item.median = price.median || 0;
    item.detailsUrl = price.detailsUrl;
  }
  return item;
}

export function findPrice<T extends IExternalPrice>(array: T[], priceToFind: T) {
  return array.find(
    (x) =>
      x.name === priceToFind.name &&
      x.quality === priceToFind.quality &&
      x.links === priceToFind.links &&
      x.level === priceToFind.level &&
      x.corrupted === priceToFind.corrupted &&
      x.frameType === priceToFind.frameType &&
      x.variant === priceToFind.variant &&
      x.elder === priceToFind.elder &&
      x.shaper === priceToFind.shaper &&
      x.ilvl === priceToFind.ilvl &&
      x.icon === priceToFind.icon &&
      x.tier === priceToFind.tier
  );
}

export function findPriceForItem(array: IExternalPrice[], priceToFind: IPricedItem) {
  return array.find(
    (x) =>
      x.name === priceToFind.name &&
      x.quality === priceToFind.quality &&
      x.links === priceToFind.links &&
      x.level === priceToFind.level &&
      x.corrupted === priceToFind.corrupted &&
      x.frameType === priceToFind.frameType &&
      x.variant === priceToFind.variant &&
      x.elder === priceToFind.elder &&
      x.shaper === priceToFind.shaper &&
      x.ilvl === priceToFind.ilvl &&
      x.icon === priceToFind.icon &&
      x.tier === priceToFind.tier
  );
}

export function getRawPriceFromPricedItem(item: IPricedItem): IExternalPrice {
  return {
    calculated: item.calculated,
    name: item.name,
    icon: item.icon,
    quality: item.quality,
    links: item.links,
    level: item.level,
    corrupted: item.corrupted,
    frameType: item.frameType,
    variant: item.variant,
    elder: item.elder,
    shaper: item.shaper,
    ilvl: item.ilvl,
    tier: item.tier,
    count: 1,
  };
}

export function mapApiPricedItemToPricedItem(item: IPricedItem) {
  return { id: item.itemId, ...item } as IPricedItem;
}

export function excludeLegacyMaps(prices: IExternalPrice[]) {
  // maps from older atlas generations linger on poe.ninja as ", Gen-N" variants;
  // only keep the latest generation present in the price list
  const genRegex = /^, Gen-(\d+)$/;
  let latestGen = 0;
  prices.forEach((p) => {
    const match = genRegex.exec(p.variant ?? '');
    if (match && +match[1] > latestGen) {
      latestGen = +match[1];
    }
  });
  return prices.filter((p) => {
    const match = genRegex.exec(p.variant ?? '');
    return !match || +match[1] === latestGen;
  });
}

export function excludeInvalidItems(prices: IExternalPrice[]) {
  const invalidItems = ['Charged Compass'];
  return prices.filter((p) => !invalidItems.includes(p.name));
}
