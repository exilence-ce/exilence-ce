import { AxiosError, AxiosResponse } from 'axios';
import axios from 'axios-observable';
import { from, Observable, throwError } from 'rxjs';
import { catchError, concatMap, map } from 'rxjs/operators';
import { rootStore } from '..';
import { ICharacterListResponse, ICharacterResponse } from '../interfaces/character.interface';
import { IGithubRelease } from '../interfaces/github/github-release.interface';
import { ILeague } from '../interfaces/league.interface';
import { IPoeProfile } from '../interfaces/poe-profile.interface';
import { IStash, IStashTab, IStashTabResponse } from '../interfaces/stash.interface';
import { RateLimitStore } from '../store/rateLimitStore';
import AppConfig from './../config/app.config';
const apiUrl = AppConfig.pathOfExileApiUrl;

export const externalService = {
  getLatestRelease,
  getStashTabs,
  getStashTabWithChildren,
  getLeagues,
  getCharacters,
  getCharacter,
  getProfile,
  loginWithOAuth,
};

/* #region github.com */
function getLatestRelease() {
  return axios.get<IGithubRelease>(
    'https://api.github.com/repos/exilence-ce/exilence-ce/releases/latest'
  );
}

function loginWithOAuth(code: string): Observable<AxiosResponse<any>> {
  return axios.get(`${AppConfig.baseUrl}/api/authentication/oauth2?code=${code}`);
}
/* #endregion */

function getStashTabs(league: string): Observable<AxiosResponse<IStash>> {
  return from(RateLimitStore.waitMulti(rootStore.rateLimitStore.stashListLimit)).pipe(
    concatMap(() => {
      return axios.get<IStash>(`${apiUrl}/stash/${league}`).pipe(
        map((leagues) => {
          RateLimitStore.adjustRateLimits(rootStore.rateLimitStore.stashListLimit, leagues.headers);
          return leagues;
        })
      );
    }),
    catchError((e: AxiosError) => {
      const err = e as AxiosError;
      if (err.response) {
        RateLimitStore.adjustRateLimits(
          rootStore.rateLimitStore.stashListLimit,
          e.response?.headers
        );
      }
      return throwError(e);
    })
  );
}

function getStashTabWithChildren(tab: IStashTab, league: string, children?: boolean) {
  return from(RateLimitStore.waitMulti(rootStore.rateLimitStore.stashLimit)).pipe(
    concatMap(() => {
      const prefix = tab.parent && children ? `${tab.parent}/` : '';
      const id = `${prefix}${tab.id}`;
      return axios.get<IStashTabResponse>(`${apiUrl}/stash/${league}/${id}`).pipe(
        map((stashTab) => {
          if (!children) {
            rootStore.uiStateStore.incrementStatusMessageCount();
          }
          RateLimitStore.adjustRateLimits(rootStore.rateLimitStore.stashLimit, stashTab.headers);
          return stashTab.data.stash;
        })
      );
    }),
    catchError((e: AxiosError) => {
      const err = e as AxiosError;
      if (err.response) {
        RateLimitStore.adjustRateLimits(rootStore.rateLimitStore.stashLimit, e.response?.headers);
      }
      return throwError(e);
    })
  );
}

function getLeagues(type: string = 'main', compact: number = 1, realm: string = 'pc') {
  return from(RateLimitStore.waitMulti(rootStore.rateLimitStore.ladderViewLimit)).pipe(
    concatMap(() => {
      const parameters = `?type=${type}&compact=${compact}${getRealmParam(realm)}`;
      return axios
        .get<ILeague[]>(apiUrl + '/leagues' + parameters, { headers: null })
        .pipe(
          map((leagues) => {
            RateLimitStore.adjustRateLimits(
              rootStore.rateLimitStore.ladderViewLimit,
              leagues.headers
            );
            return leagues.data;
          })
        );
    }),
    catchError((e: AxiosError) => {
      const err = e as AxiosError;
      if (err.response) {
        RateLimitStore.adjustRateLimits(
          rootStore.rateLimitStore.ladderViewLimit,
          e.response?.headers
        );
      }
      return throwError(e);
    })
  );
}

function getCharacters() {
  return from(RateLimitStore.waitMulti(rootStore.rateLimitStore.characterListLimit)).pipe(
    concatMap(() => {
      return axios.get<ICharacterListResponse>(`${apiUrl}/character`).pipe(
        map((characters) => {
          RateLimitStore.adjustRateLimits(
            rootStore.rateLimitStore.characterListLimit,
            characters.headers
          );
          return characters.data;
        }),
        catchError((e: AxiosError) => {
          const err = e as AxiosError;
          if (err.response) {
            RateLimitStore.adjustRateLimits(
              rootStore.rateLimitStore.characterListLimit,
              e.response?.headers
            );
          }
          return throwError(e);
        })
      );
    })
  );
}

function getCharacter(character: string) {
  return from(RateLimitStore.waitMulti(rootStore.rateLimitStore.characterLimit)).pipe(
    concatMap(() => {
      return axios.get<ICharacterResponse>(`${apiUrl}/character/${character}`).pipe(
        map((character) => {
          RateLimitStore.adjustRateLimits(
            rootStore.rateLimitStore.characterLimit,
            character.headers
          );
          return character.data;
        })
      );
    }),
    catchError((e: AxiosError) => {
      const err = e as AxiosError;
      if (err.response) {
        RateLimitStore.adjustRateLimits(
          rootStore.rateLimitStore.characterLimit,
          e.response?.headers
        );
      }
      return throwError(e);
    })
  );
}

// No rate-limiting
function getProfile(realm: string = 'pc'): Observable<AxiosResponse<IPoeProfile>> {
  const parameters = `?realm=${realm}`;
  return axios.get<IPoeProfile>(apiUrl + '/profile' + parameters);
}

function getRealmParam(realm?: string) {
  return realm !== undefined ? `&realm=${realm}` : '';
}

/* #endregion */
