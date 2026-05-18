import { getAuthCallbackPayload } from './auth-callback.utils';

describe('getAuthCallbackPayload', () => {
  it('parses code and state from a custom protocol callback', () => {
    expect(getAuthCallbackPayload('exilence://auth?code=abc123&state=csrf-token')).toEqual({
      code: 'abc123',
      error: undefined,
      state: 'csrf-token',
    });
  });

  it('parses the callback URL from Windows second-instance arguments', () => {
    expect(
      getAuthCallbackPayload([
        'C:\\Program Files\\Exilence CE\\Exilence CE.exe',
        '--some-electron-flag',
        'exilence://auth?code=abc123&state=csrf-token',
      ])
    ).toEqual({
      code: 'abc123',
      error: undefined,
      state: 'csrf-token',
    });
  });

  it('decodes authorization errors from fallback query parsing', () => {
    expect(getAuthCallbackPayload('not-a-url ?error=access+denied&state=csrf-token')).toEqual({
      code: undefined,
      error: 'access denied',
      state: 'csrf-token',
    });
  });
});
