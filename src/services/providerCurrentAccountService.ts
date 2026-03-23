import { invoke } from '@tauri-apps/api/core';

export type ProviderCurrentPlatform =
  | 'windsurf'
  | 'kiro'
  | 'cursor'
  | 'gemini'
  | 'codebuddy'
  | 'codebuddy_cn'
  | 'qoder'
  | 'trae'
  | 'workbuddy'
  | 'zed';

export async function getProviderCurrentAccountId(
  platform: ProviderCurrentPlatform,
): Promise<string | null> {
  return await invoke('get_provider_current_account_id', { platform });
}
