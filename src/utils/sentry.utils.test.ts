import type { Event } from '@sentry/types';
import { sanitizeSentryEvent } from './sentry.utils';

describe('sanitizeSentryEvent', () => {
  it('redacts auth headers, cookies, and OAuth query parameters', () => {
    const event: Event = {
      message: 'request failed https://api.exilence.de/oauth?code=secret-code&state=secret-state',
      request: {
        url: 'https://api.exilence.de/oauth?code=secret-code&state=secret-state&safe=1',
        headers: {
          Authorization: 'Bearer secret-token',
          Cookie: 'POESESSID=secret-cookie',
          Accept: 'application/json',
        },
        cookies: {
          POESESSID: 'secret-cookie',
        },
        query_string: {
          code: 'secret-code',
          safe: '1',
        },
      },
    };

    expect(sanitizeSentryEvent(event)).toEqual({
      message: 'request failed https://api.exilence.de/oauth?code=[Filtered]&state=[Filtered]',
      request: {
        url: 'https://api.exilence.de/oauth?code=%5BFiltered%5D&state=%5BFiltered%5D&safe=1',
        headers: {
          Authorization: '[Filtered]',
          Cookie: '[Filtered]',
          Accept: 'application/json',
        },
        cookies: undefined,
        query_string: {
          code: '[Filtered]',
          safe: '1',
        },
      },
    });
  });

  it('redacts bearer tokens from string request data', () => {
    const event: Event = {
      request: {
        data: 'Authorization: Bearer secret-token',
      },
    };

    expect(sanitizeSentryEvent(event).request!.data).toBe('Authorization: Bearer [Filtered]');
  });
});
