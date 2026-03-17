import axios, { AxiosResponse } from 'axios';
import { forkJoin, from, of } from 'rxjs';
import RateLimiter from 'rxjs-ratelimiter';
import { catchError, map } from 'rxjs/operators';
import { IExternalPrice } from '../interfaces/external-price.interface';
import { IPoeNinjaExchangeOverview } from '../interfaces/poe-ninja/poe-ninja-exchange-overview.interface';
import { IPoeNinjaItemOverview } from '../interfaces/poe-ninja/poe-ninja-item-overview.interface';
import {
  getExternalPriceFromNinjaExchangeItem,
  getExternalPriceFromNinjaItem,
} from '../utils/price.utils';

const rateLimiter = new RateLimiter(1, 1);
const exchangeRateLimiter = new RateLimiter(1, 2);
const apiUrl = 'https://poe.ninja/api/data';
const exchangeApiUrl = 'https://poe.ninja/poe1/api/economy/exchange/current/overview';

export const poeninjaService = {
  getExchangeCategories,
  getItemCategories,
  getItemCategoryOverview,
  getExchangeCategoryOverview,
  getItemPrices,
  getExchangePrices,
};

function getExchangeCategories() {
  return [
    'Currency',
    'Fragment',
    'Runegraft',
    'AllflameEmber',
    'Tattoo',
    'Omen',
    'DjinnCoin',
    'DivinationCard',
    'Oil',
    'Artifact',
    'Scarab',
    'DeliriumOrb',
    'Essence',
    'Resonator',
    'Fossil',
  ];
}

function getItemCategories() {
  // commented categories are mostly in accuracy pricing
  const categories = [
    'Incubator',
    //'Prophecy',
    'SkillGem',
    'UniqueMap',
    'Map',
    'UniqueJewel',
    'UniqueFlask',
    'UniqueWeapon',
    'UniqueArmour',
    'UniqueRelic',
    //'Watchstone',
    'UniqueAccessory',
    'Beast',
    'Vial',
    'Invitation',
    'Memory',
    //'ClusterJewel',
    'BlightedMap',
    'BlightRavagedMap',
    'Coffin',
    //'BaseType',
    //'HelmetEnchant',
  ];
  return categories;
}

function getItemCategoryOverview(league: string, type: string) {
  const parameters = `?league=${league}&type=${type}`;
  return rateLimiter.limit(
    from(axios.get<IPoeNinjaItemOverview>(`${apiUrl}/itemoverview${parameters}`))
  );
}

function getExchangeCategoryOverview(league: string, type: string) {
  const parameters = `?league=${league}&type=${type}`;
  return exchangeRateLimiter.limit(
    from(axios.get<IPoeNinjaExchangeOverview>(`${exchangeApiUrl}${parameters}`))
  );
}

function getItemPrices(league: string) {
  return forkJoin(
    getItemCategories().map((type) => {
      return getItemCategoryOverview(league, type).pipe(
        map((response: AxiosResponse<IPoeNinjaItemOverview>) => {
          if (response.data) {
            return response.data.lines.map((lines) => {
              return getExternalPriceFromNinjaItem(lines, type, league) as IExternalPrice;
            });
          } else {
            return []; // no prices found on ninja
          }
        })
      );
    })
  ).pipe(map((arrays) => arrays.reduce((acc, array) => [...acc, ...array], [])));
}

function getExchangePrices(league: string) {
  return forkJoin(
    getExchangeCategories().map((type) => {
      return getExchangeCategoryOverview(league, type).pipe(
        map((response: AxiosResponse<IPoeNinjaExchangeOverview>) => {
          if (response.data && response.data.lines) {
            const itemMap = new Map(response.data.items.map((i) => [i.id, i]));
            return response.data.lines.map((line) => {
              const item = itemMap.get(line.id);
              return getExternalPriceFromNinjaExchangeItem(
                line,
                item,
                type,
                league
              ) as IExternalPrice;
            });
          } else {
            return [];
          }
        }),
        catchError(() => of([] as IExternalPrice[]))
      );
    })
  ).pipe(map((arrays) => arrays.reduce((acc, array) => [...acc, ...array], [])));
}
