export type StartupTheme = 'light' | 'dark' | 'system';

export interface StartupAppearance {
  theme: StartupTheme;
  uiScale: number;
  topRightAdVisible: boolean;
  savedAt: number;
}

export const STARTUP_APPEARANCE_CACHE_KEY = 'agtools.startup_appearance.v1';

const DEFAULT_APPEARANCE: StartupAppearance = {
  theme: 'system',
  uiScale: 1,
  topRightAdVisible: true,
  savedAt: 0,
};

function normalizeTheme(value: unknown): StartupTheme {
  return value === 'light' || value === 'dark' || value === 'system' ? value : 'system';
}

function normalizeUiScale(value: unknown): number {
  const scale = typeof value === 'number' ? value : Number.parseFloat(String(value ?? ''));
  return Number.isFinite(scale) ? Math.min(2, Math.max(0.8, scale)) : 1;
}

export function resolveEffectiveTheme(theme: StartupTheme): 'light' | 'dark' {
  if (theme === 'system') {
    return window.matchMedia?.('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  return theme;
}

export function loadStartupAppearance(): StartupAppearance {
  if (typeof localStorage === 'undefined') {
    return DEFAULT_APPEARANCE;
  }

  try {
    const raw = localStorage.getItem(STARTUP_APPEARANCE_CACHE_KEY);
    if (!raw) {
      return DEFAULT_APPEARANCE;
    }
    const parsed = JSON.parse(raw) as Partial<StartupAppearance>;
    return {
      theme: normalizeTheme(parsed.theme),
      uiScale: normalizeUiScale(parsed.uiScale),
      topRightAdVisible:
        typeof parsed.topRightAdVisible === 'boolean'
          ? parsed.topRightAdVisible
          : DEFAULT_APPEARANCE.topRightAdVisible,
      savedAt: typeof parsed.savedAt === 'number' ? parsed.savedAt : 0,
    };
  } catch {
    return DEFAULT_APPEARANCE;
  }
}

export function persistStartupAppearance(
  appearance: Partial<Omit<StartupAppearance, 'savedAt'>>,
): StartupAppearance {
  const current = loadStartupAppearance();
  const next: StartupAppearance = {
    ...current,
    ...appearance,
    theme: normalizeTheme(appearance.theme ?? current.theme),
    uiScale: normalizeUiScale(appearance.uiScale ?? current.uiScale),
    topRightAdVisible:
      typeof appearance.topRightAdVisible === 'boolean'
        ? appearance.topRightAdVisible
        : current.topRightAdVisible,
    savedAt: Date.now(),
  };

  try {
    localStorage.setItem(STARTUP_APPEARANCE_CACHE_KEY, JSON.stringify(next));
  } catch {
    // Startup appearance cache is best effort.
  }

  return next;
}

export function applyStartupTheme(theme: StartupTheme): 'light' | 'dark' {
  const effectiveTheme = resolveEffectiveTheme(theme);
  document.documentElement.setAttribute('data-theme', effectiveTheme);
  document.documentElement.style.colorScheme = effectiveTheme;
  return effectiveTheme;
}

export function applyCachedStartupAppearance(): StartupAppearance {
  const appearance = loadStartupAppearance();
  applyStartupTheme(appearance.theme);
  return appearance;
}

