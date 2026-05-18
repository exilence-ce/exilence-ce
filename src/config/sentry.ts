import * as Sentry from '@sentry/browser';

import AppConfig from './app.config';
import { sanitizeSentryEvent } from '../utils/sentry.utils';

function initSentry() {
  if (AppConfig.production) {
    Sentry.init({
      dsn: AppConfig.sentryBrowserDsn,
      beforeSend: sanitizeSentryEvent,
      ignoreErrors: [
        'Request failed',
        'net::',
        'Network Error',
        'HttpError',
        AppConfig.pathOfExileUrl,
        AppConfig.pathOfExileApiUrl,
        AppConfig.poeNinjaBaseUrl,
        AppConfig.githubBaseUrl,
      ],
    });
  }
}

export default initSentry;
