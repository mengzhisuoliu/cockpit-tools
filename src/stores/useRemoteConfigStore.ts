import { create } from 'zustand';
import type { PlatformId } from '../types/platform';
import type { RemoteConfigState } from '../types/remoteConfig';
import {
  forceRefreshRemoteConfigState,
  getRemoteConfigState,
} from '../services/remoteConfigService';

const DEFAULT_REFRESH_INTERVAL_MS = 60 * 60 * 1000;
const REMOTE_CONFIG_STATE_CACHE_KEY = 'agtools.remote_config_state.cache.v1';

const EMPTY_STATE: RemoteConfigState = {
  version: '',
  updatedAt: 0,
  currentOs: '',
  hiddenPlatformIds: [],
  appliedRules: [],
  refreshIntervalMs: DEFAULT_REFRESH_INTERVAL_MS,
};

interface RemoteConfigStoreState {
  state: RemoteConfigState;
  hiddenPlatformIds: PlatformId[];
  loading: boolean;
  initialized: boolean;
  lastError: string | null;
  fetchState: (force?: boolean) => Promise<RemoteConfigState>;
}

function isRemoteConfigState(value: unknown): value is RemoteConfigState {
  if (!value || typeof value !== 'object') {
    return false;
  }
  const record = value as Partial<RemoteConfigState>;
  return Array.isArray(record.hiddenPlatformIds);
}

function normalizeRemoteConfigState(state: RemoteConfigState): RemoteConfigState {
  return {
    ...EMPTY_STATE,
    ...state,
    hiddenPlatformIds: Array.isArray(state.hiddenPlatformIds) ? state.hiddenPlatformIds : [],
    appliedRules: Array.isArray(state.appliedRules) ? state.appliedRules : [],
    refreshIntervalMs:
      typeof state.refreshIntervalMs === 'number' && Number.isFinite(state.refreshIntervalMs)
        ? state.refreshIntervalMs
        : DEFAULT_REFRESH_INTERVAL_MS,
  };
}

function loadCachedRemoteConfigState(): RemoteConfigState {
  if (typeof localStorage === 'undefined') {
    return EMPTY_STATE;
  }

  try {
    const raw = localStorage.getItem(REMOTE_CONFIG_STATE_CACHE_KEY);
    if (!raw) return EMPTY_STATE;
    const parsed = JSON.parse(raw) as { state?: unknown };
    return isRemoteConfigState(parsed.state)
      ? normalizeRemoteConfigState(parsed.state)
      : EMPTY_STATE;
  } catch {
    return EMPTY_STATE;
  }
}

function persistRemoteConfigState(state: RemoteConfigState): void {
  if (typeof localStorage === 'undefined') {
    return;
  }

  try {
    localStorage.setItem(
      REMOTE_CONFIG_STATE_CACHE_KEY,
      JSON.stringify({ savedAt: Date.now(), state: normalizeRemoteConfigState(state) }),
    );
  } catch {
    // Cache writes are best effort only.
  }
}

const initialRemoteConfigState = loadCachedRemoteConfigState();

export const useRemoteConfigStore = create<RemoteConfigStoreState>((set, get) => ({
  state: initialRemoteConfigState,
  hiddenPlatformIds: initialRemoteConfigState.hiddenPlatformIds,
  loading: false,
  initialized: Boolean(initialRemoteConfigState.version || initialRemoteConfigState.updatedAt),
  lastError: null,

  fetchState: async (force = false) => {
    set({ loading: true });
    try {
      const nextState = normalizeRemoteConfigState(
        force
          ? await forceRefreshRemoteConfigState()
          : await getRemoteConfigState(),
      );
      set({
        state: nextState,
        hiddenPlatformIds: nextState.hiddenPlatformIds,
        loading: false,
        initialized: true,
        lastError: null,
      });
      persistRemoteConfigState(nextState);
      return nextState;
    } catch (error) {
      console.error('Failed to load remote config:', error);
      set((current) => ({
        loading: false,
        initialized: true,
        lastError: String(error),
        hiddenPlatformIds: current.state.hiddenPlatformIds,
      }));
      return get().state;
    }
  },
}));

