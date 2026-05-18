export interface IAuthCallbackPayload {
  code?: string;
  error?: string;
  state?: string;
}

export function getAuthCallbackPayload(deepLink: string | string[]): IAuthCallbackPayload {
  const link = Array.isArray(deepLink)
    ? deepLink.find((arg) => arg.includes('code=') || arg.includes('error=')) ?? deepLink.join(' ')
    : deepLink;

  return {
    code: getDeepLinkParam(link, 'code'),
    error: getDeepLinkParam(link, 'error'),
    state: getDeepLinkParam(link, 'state'),
  };
}

function getDeepLinkParam(link: string, name: string): string | undefined {
  try {
    return new URL(link).searchParams.get(name) ?? undefined;
  } catch (_error) {
    const escapedName = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const match = new RegExp(`[?&]${escapedName}=([^&#]*)`).exec(link);
    return match && match.length > 1 ? decodeParam(match[1]) : undefined;
  }
}

function decodeParam(value: string): string {
  try {
    return decodeURIComponent(value.replace(/\+/g, ' '));
  } catch (_error) {
    return value;
  }
}
