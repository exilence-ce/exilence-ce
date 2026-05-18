import { IExternalPrice } from '../interfaces/external-price.interface';
import { IPricedItem } from '../interfaces/priced-item.interface';

const normalizeVariant = (variant?: string) =>
  (variant ?? '')
    .toLowerCase()
    .replace(/passives/g, 'passive')
    .replace(/\s+/g, ' ')
    .trim();

export const isVariantMatch = (itemVariant?: string, priceVariant?: string) => {
  const normalizedItemVariant = normalizeVariant(itemVariant);
  const normalizedPriceVariant = normalizeVariant(priceVariant);

  return (
    normalizedItemVariant === normalizedPriceVariant ||
    normalizedItemVariant === '' ||
    normalizedPriceVariant === ''
  );
};

export const isUniqueLinkMatch = (itemLinks: number, priceLinks?: number) => {
  const normalizedPriceLinks = priceLinks ?? 0;
  return itemLinks < 5 ? normalizedPriceLinks < 5 : normalizedPriceLinks === itemLinks;
};

export const filterUniquePriceCandidates = (item: IPricedItem, prices: IExternalPrice[]) =>
  prices.filter(
    (price) =>
      price.name === item.name &&
      isUniqueLinkMatch(item.links, price.links) &&
      price.frameType === 3 &&
      isVariantMatch(item.variant, price.variant)
  );

export const pickBestUniquePrice = (
  item: IPricedItem,
  prices: IExternalPrice[]
): IExternalPrice | undefined => {
  const itemPrices = filterUniquePriceCandidates(item, prices);
  const qualityPrices = itemPrices.filter((ip) => !ip.quality || ip.quality === item.quality);

  if (qualityPrices.length === 1) {
    return qualityPrices[0];
  }

  const exactVariantPrices = qualityPrices.filter(
    (ip) => normalizeVariant(ip.variant) === normalizeVariant(item.variant)
  );
  if (exactVariantPrices.length === 1) {
    return exactVariantPrices[0];
  }

  const unqualifiedPrices = itemPrices.filter((ip) => !ip.quality || ip.quality === 0);
  if (unqualifiedPrices.length === 1) {
    return unqualifiedPrices[0];
  }

  return undefined;
};
