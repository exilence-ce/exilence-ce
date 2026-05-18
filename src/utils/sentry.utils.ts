import type { Event } from '@sentry/types';

const FILTERED = '[Filtered]';
const SENSITIVE_HEADERS = ['authorization', 'cookie', 'set-cookie'];
const SENSITIVE_QUERY_PARAMS = ['access_token', 'code', 'refresh_token', 'state', 'token'];

export function sanitizeSentryEvent(event: Event): Event {
  if (event.message) {
    event.message = redactSensitiveValues(event.message);
  }

  if (event.request) {
    event.request.url = redactUrl(event.request.url);

    if (typeof event.request.data === 'string') {
      event.request.data = redactSensitiveValues(event.request.data);
    }

    if (typeof event.request.query_string === 'string') {
      event.request.query_string = redactSensitiveValues(event.request.query_string);
    } else if (Array.isArray(event.request.query_string)) {
      event.request.query_string = event.request.query_string.map(([key, value]): [
        string,
        string
      ] => [key, isSensitiveQueryParam(key) ? FILTERED : redactSensitiveValues(value)]);
    } else if (event.request.query_string) {
      const queryString = event.request.query_string;
      Object.keys(queryString).forEach((key) => {
        queryString[key] = isSensitiveQueryParam(key)
          ? FILTERED
          : redactSensitiveValues(queryString[key]);
      });
    }

    redactHeaders(event.request.headers);
    event.request.cookies = undefined;
  }

  return event;
}

function redactHeaders(headers?: Record<string, string>): void {
  if (!headers) {
    return;
  }

  Object.keys(headers).forEach((header) => {
    if (SENSITIVE_HEADERS.includes(header.toLowerCase())) {
      headers[header] = FILTERED;
    }
  });
}

function redactUrl(url?: string): string | undefined {
  if (!url) {
    return url;
  }

  try {
    const parsedUrl = new URL(url);
    SENSITIVE_QUERY_PARAMS.forEach((param) => {
      if (parsedUrl.searchParams.has(param)) {
        parsedUrl.searchParams.set(param, FILTERED);
      }
    });
    return parsedUrl.toString();
  } catch (_error) {
    return redactSensitiveValues(url);
  }
}

function redactSensitiveValues(value: string): string {
  const withoutBearerToken = value.replace(/Bearer\s+[^,&\s]+/gi, `Bearer ${FILTERED}`);
  return SENSITIVE_QUERY_PARAMS.reduce((current, param) => {
    return current.replace(new RegExp(`([?&]${param}=)[^&#\\s,]+`, 'gi'), `$1${FILTERED}`);
  }, withoutBearerToken);
}

function isSensitiveQueryParam(param: string): boolean {
  return SENSITIVE_QUERY_PARAMS.includes(param.toLowerCase());
}
