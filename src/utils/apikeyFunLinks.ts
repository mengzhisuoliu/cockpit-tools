export const APIKEY_FUN_REGISTER_URL = 'https://apikey.fun/register?aff=cockpit';
export const APIKEY_FUN_DOCS_URL = 'https://apikey.fun/docs';
export const APIKEY_FUN_GLOBAL_ENDPOINT = 'https://api.apikey.fun';
export const APIKEY_FUN_DIRECT_ENDPOINT = 'https://slb.apikey.fun';
export const APIKEY_FUN_SOURCE_TAG = 'apikey_fun';

export function buildApiKeyFunProviderBaseUrl(endpoint: string): string {
  return `${endpoint.trim().replace(/\/+$/, '')}/v1`;
}

export function normalizeApiKeyFunOfficialUrl(value?: string | null): string {
  const raw = value?.trim() ?? '';
  if (!raw) return '';
  try {
    const parsed = new URL(raw);
    if (
      parsed.protocol === 'https:' &&
      parsed.hostname.toLowerCase() === 'apikey.fun' &&
      (parsed.pathname === '/' || parsed.pathname === '/register')
    ) {
      return APIKEY_FUN_REGISTER_URL;
    }
  } catch {
    return raw;
  }
  return raw;
}
