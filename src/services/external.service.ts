import { AxiosError, AxiosResponse } from 'axios';
import axios from 'axios-observable';
import moment from 'moment';
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

type LatencyListKeys = keyof typeof rootStore.rateLimitStore.snapshotLatencyList;
export class GeneralTotalTimer {
  start: moment.Moment;
  constructor() {
    this.start = moment.utc();
  }
  stopAndWrite(latency: number) {
    const time = moment.utc().diff(this.start);
    rootStore.rateLimitStore.timeEstimation?.measuredTime.push(time - latency);
  }
}
export class GeneralLatencyTimer {
  start: moment.Moment;
  constructor(public latencyName: LatencyListKeys) {
    this.start = moment.utc();
  }
  stopAndWrite() {
    const latency = moment.utc().diff(this.start);
    rootStore.rateLimitStore.snapshotLatencyList[this.latencyName] = latency;
    rootStore.rateLimitStore.timeEstimation?.measuredLatencys.push(latency);
    return latency;
  }
}
export class StashLatencyTimer extends GeneralLatencyTimer {
  latencyMeasureInterval: number = 1;
  constructor() {
    super('stash-request-limit');
    this.setLatencyMeasureInterval();
  }
  setLatencyMeasureInterval() {
    if (rootStore.uiStateStore.statusMessage?.totalCount || 1 > 1) {
      this.latencyMeasureInterval = 2; // Measure every second request
    } else {
      this.latencyMeasureInterval = 1;
    }
  }
  stopAndWrite() {
    const latency = moment.utc().diff(this.start);
    rootStore.rateLimitStore.timeEstimation?.measuredLatencys.push(latency);
    if ((rootStore.uiStateStore.currentRequest || 0) % 30 === this.latencyMeasureInterval) {
      rootStore.rateLimitStore.snapshotLatencyList[this.latencyName] = latency;
    }
    return latency;
  }
  restart() {
    this.start = moment.utc();
    this.setLatencyMeasureInterval();
  }
}

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
  const totalTimer = new GeneralTotalTimer();
  rootStore.rateLimitStore.setEstimatedSnapshotTime();
  return from(RateLimitStore.waitMulti(rootStore.rateLimitStore.stashListLimit)).pipe(
    concatMap(() => {
      const latencyTimer = new GeneralLatencyTimer('stash-list-request-limit');
      return axios.get<IStash>(`${apiUrl}/stash/${league}`).pipe(
        map((leagues) => {
          const latency = latencyTimer.stopAndWrite();
          totalTimer.stopAndWrite(latency);
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
          err.response.headers
        );
      }
      return throwError(e);
    })
  );
}

function getStashTabWithChildren(tab: IStashTab, league: string, children?: boolean) {
  const totalTimer = new GeneralTotalTimer();
  rootStore.rateLimitStore.setEstimatedSnapshotTime();
  return from(RateLimitStore.waitMulti(rootStore.rateLimitStore.stashLimit)).pipe(
    concatMap(() => {
      if (rootStore.uiStateStore.currentRequest !== undefined) {
        rootStore.uiStateStore.currentRequest++;
        console.log(`Increased currentRequest count to ${rootStore.uiStateStore.currentRequest}`);
      }
      // Measure first and every 30th request
      if (rootStore.rateLimitStore.stashLatencyTimer) {
        rootStore.rateLimitStore.stashLatencyTimer.restart();
      } else {
        rootStore.rateLimitStore.stashLatencyTimer = new StashLatencyTimer();
      }

      const prefix = tab.parent && children ? `${tab.parent}/` : '';
      const id = `${prefix}${tab.id}`;
      return axios.get<IStashTabResponse>(`${apiUrl}/stash/${league}/${id}`).pipe(
        map((stashTab) => {
          rootStore.uiStateStore.incrementStatusMessageCount();
          const latency = rootStore.rateLimitStore.stashLatencyTimer?.stopAndWrite() || 0;
          totalTimer.stopAndWrite(latency);
          RateLimitStore.adjustRateLimits(rootStore.rateLimitStore.stashLimit, stashTab.headers);
          return stashTab.data.stash;
        })
      );
    }),
    catchError((e: AxiosError) => {
      const err = e as AxiosError;
      if (err.response) {
        RateLimitStore.adjustRateLimits(rootStore.rateLimitStore.stashLimit, err.response.headers);
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
          err.response.headers
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
              err.response.headers
            );
          }
          return throwError(e);
        })
      );
    })
  );
}

function getCharacter(character: string) {
  // const totalTimer = new GeneralTotalTimer();
  return from(RateLimitStore.waitMulti(rootStore.rateLimitStore.characterLimit)).pipe(
    concatMap(() => {
      // const latencyTimer = new GeneralLatencyTimer('character-request-limit');
      return axios.get<ICharacterResponse>(`${apiUrl}/character/${character}`).pipe(
        map((character) => {
          // const latency = latencyTimer.stopAndWrite();
          // totalTimer.stopAndWrite(latency);
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
          err.response.headers
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
