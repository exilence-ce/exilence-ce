/* eslint-disable no-undef */
// const { getStashTabWithChildren, getStashTab } = require('../../../services/external.service');
const { RateLimiter } = require('../../../utils/rateLimiter.utils');
const {
  RATE_LIMIT_POLICIES,
  preventQueueCreation,
  adjustRateLimits,
} = require('../../rateLimitStore');
const { from, Observable, of, throwError } = require('rxjs');
const { catchError, concatMap, delay, map, mergeMap } = require('rxjs/operators');
import axios from 'axios';

function getStashTab() {
  return axios.get(`https://api.pathofexile.com/stash/Ancestor/a3e5ec6098`);
}

test('RateLimiting', async () => {
  axios.defaults.headers.common[
    'Authorization'
  ] = `Bearer a48a4761dc6c0f250f9c1ae4dc325163456b44b0`;
  try {
    for (let i = 0; i < 10; i++) {
      // from(RateLimiter.waitMulti(RATE_LIMIT_POLICIES.FETCH)).pipe(
      //   mergeMap(() => {
      //     return getStashTab('Ancestor', `a3e5ec6098`).pipe(
      //       map((stashTab) => {
      //         adjustRateLimits(RATE_LIMIT_POLICIES.FETCH, stashTab.headers);
      //         return stashTab.data.stash;
      //       })
      //     );
      //   })
      // );

      await RateLimiter.waitMulti(RATE_LIMIT_POLICIES.FETCH);
      const res = await getStashTab();
      console.log(res.headers);
      adjustRateLimits(RATE_LIMIT_POLICIES.FETCH, res.headers);
    }
  } catch (error) {
    console.log(error.response.data);
  }
});
