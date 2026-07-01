import { create } from 'zustand';
import type { SponsorModuleState } from '../types/sponsor';
import { forceRefreshSponsorModuleState, getSponsorModuleState } from '../services/sponsorService';

const EMPTY_STATE: SponsorModuleState = {
  sponsorModule: null,
};

const SPONSOR_STATE_CACHE_KEY = 'agtools.sponsor_state.cache.v1';

interface SponsorStoreState {
  state: SponsorModuleState;
  loading: boolean;
  initialized: boolean;
  fetchState: (force?: boolean) => Promise<SponsorModuleState>;
}

function isSponsorModuleState(value: unknown): value is SponsorModuleState {
  return Boolean(value && typeof value === 'object' && 'sponsorModule' in value);
}

function loadCachedSponsorState(): SponsorModuleState {
  if (typeof localStorage === 'undefined') {
    return EMPTY_STATE;
  }

  try {
    const raw = localStorage.getItem(SPONSOR_STATE_CACHE_KEY);
    if (!raw) return EMPTY_STATE;
    const parsed = JSON.parse(raw) as { state?: unknown };
    return isSponsorModuleState(parsed.state) ? parsed.state : EMPTY_STATE;
  } catch {
    return EMPTY_STATE;
  }
}

function persistSponsorState(state: SponsorModuleState): void {
  if (typeof localStorage === 'undefined') {
    return;
  }

  try {
    localStorage.setItem(SPONSOR_STATE_CACHE_KEY, JSON.stringify({ savedAt: Date.now(), state }));
  } catch {
    // Cache writes are best effort only.
  }
}

const initialSponsorState = loadCachedSponsorState();

export const useSponsorStore = create<SponsorStoreState>((set, get) => ({
  state: initialSponsorState,
  loading: false,
  initialized: Boolean(initialSponsorState.sponsorModule),

  fetchState: async (force = false) => {
    set({ loading: true });
    try {
      const nextState = force
        ? await forceRefreshSponsorModuleState()
        : await getSponsorModuleState();
      set({ state: nextState, loading: false, initialized: true });
      persistSponsorState(nextState);
      return nextState;
    } catch (error) {
      console.error('Failed to load sponsor module state:', error);
      set({ state: get().state, loading: false, initialized: true });
      return get().state;
    }
  },
}));

