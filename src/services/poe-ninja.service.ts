import axios, { AxiosError, AxiosResponse } from 'axios';
import { forkJoin, from, Observable, of, throwError } from 'rxjs';
import RateLimiter from 'rxjs-ratelimiter';
import { catchError, map } from 'rxjs/operators';
import { IExternalPrice } from '../interfaces/external-price.interface';
import {
  IPoeNinjaExchangeOverview,
  IPoeNinjaExchangeOverviewItem,
} from '../interfaces/poe-ninja/poe-ninja-exchange-overview.interface';
import { IPoeNinjaItemOverview } from '../interfaces/poe-ninja/poe-ninja-item-overview.interface';
import {
  getExternalPriceFromNinjaCurrencyItem,
  getExternalPriceFromNinjaExchangeLine,
  getExternalPriceFromNinjaItem,
} from '../utils/price.utils';
import { IPoeNinjaCurrencyOverview } from './../interfaces/poe-ninja/poe-ninja-currency-overview.interface';

const rateLimiter = new RateLimiter(1, 1);
const apiUrl = 'https://poe.ninja/poe1/api/economy';

export const poeninjaService = {
  getCurrencyCategories,
  getItemCategories,
  getExchangeCategories,
  getItemCategoryOverview,
  getCurrencyCategoryOverview,
  getExchangeCategoryOverview,
  getItemPrices,
  getCurrencyPrices,
  getExchangePrices,
};

// a category being dropped by poe.ninja (removed league mechanic) surfaces as a 404;
// treat it as "no prices" instead of failing the entire price fetch
function ignoreRemovedCategory<T>() {
  return catchError<AxiosResponse<T>, Observable<AxiosResponse<T>>>((e: AxiosError) =>
    e.response && e.response.status === 404
      ? of(({ data: undefined } as unknown) as AxiosResponse<T>)
      : throwError(e)
  );
}

function getCurrencyCategories() {
  const categories = ['Currency', 'Fragment'];
  return categories;
}

// categories that poe.ninja serves through the currency-exchange overview.
// Currency and Fragment are also fetched here since the stash overview only
// covers a subset of them (e.g no Exalted Orb); duplicates are removed in
// the price store, preferring the stash overview entry
function getExchangeCategories() {
  const categories = [
    'Currency',
    'Fragment',
    'DivinationCard',
    'Scarab',
    'Essence',
    'Oil',
    'Fossil',
    'Resonator',
    'DeliriumOrb',
    'Artifact',
    'Tattoo',
    'Omen',
    'AllflameEmber',
    'Runegraft',
    'DjinnCoin',
    'Astrolabe',
  ];
  return categories;
}

function getItemCategories() {
  // commented categories are mostly in accuracy pricing
  const categories = [
    'Incubator',
    'SkillGem',
    'UniqueMap',
    'Map',
    'BlightedMap',
    'BlightRavagedMap',
    'UniqueJewel',
    'UniqueFlask',
    'UniqueWeapon',
    'UniqueArmour',
    'UniqueRelic',
    'UniqueAccessory',
    'UniqueTincture',
    'Beast',
    'Vial',
    'Invitation',
    'Memory',
    //'ForbiddenJewel', overlaps with UniqueJewel
    //'ClusterJewel',
    //'BaseType',
    //'ValdoMap',
    //'IncursionTemple',
  ];
  return categories;
}

function getItemCategoryOverview(league: string, type: string) {
  const parameters = `?league=${league}&type=${type}`;
  return rateLimiter.limit(
    from(axios.get<IPoeNinjaItemOverview>(`${apiUrl}/stash/current/item/overview${parameters}`))
  );
}

function getCurrencyCategoryOverview(league: string, type: string) {
  const parameters = `?league=${league}&type=${type}`;
  return rateLimiter.limit(
    from(
      axios.get<IPoeNinjaCurrencyOverview>(`${apiUrl}/stash/current/currency/overview${parameters}`)
    )
  );
}

function getExchangeCategoryOverview(league: string, type: string) {
  const parameters = `?league=${league}&type=${type}`;
  return rateLimiter.limit(
    from(axios.get<IPoeNinjaExchangeOverview>(`${apiUrl}/exchange/current/overview${parameters}`))
  );
}

function getItemPrices(league: string) {
  return forkJoin(
    getItemCategories().map((type) => {
      return getItemCategoryOverview(league, type).pipe(
        ignoreRemovedCategory<IPoeNinjaItemOverview>(),
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

function getCurrencyPrices(league: string) {
  return forkJoin(
    getCurrencyCategories().map((type) => {
      return getCurrencyCategoryOverview(league, type).pipe(
        ignoreRemovedCategory<IPoeNinjaCurrencyOverview>(),
        map((response: AxiosResponse<IPoeNinjaCurrencyOverview>) => {
          if (response.data) {
            return response.data.lines.map((lines) => {
              const currencyDetail = response.data.currencyDetails.find(
                (detail) => detail.name === lines.currencyTypeName
              );
              return getExternalPriceFromNinjaCurrencyItem(
                lines,
                currencyDetail,
                type,
                league
              ) as IExternalPrice;
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
        ignoreRemovedCategory<IPoeNinjaExchangeOverview>(),
        map((response: AxiosResponse<IPoeNinjaExchangeOverview>) => {
          if (response.data) {
            const itemsById: { [id: string]: IPoeNinjaExchangeOverviewItem } = {};
            (response.data.items ?? []).forEach((item) => {
              itemsById[item.id] = item;
            });
            return response.data.lines.map((line) => {
              return getExternalPriceFromNinjaExchangeLine(
                line,
                itemsById[line.id],
                type,
                league
              ) as IExternalPrice;
            });
          } else {
            return []; // no prices found on ninja
          }
        })
      );
    })
  ).pipe(map((arrays) => arrays.reduce((acc, array) => [...acc, ...array], [])));
}
