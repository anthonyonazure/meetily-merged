'use client';

/**
 * Ledger theme system.
 *
 * Three-way preference (system / light / dark) that toggles the `.dark`
 * class on <html>, which drives every CSS token in globals.css.
 *
 * Persistence: localStorage (`meetily-theme`) is the fast synchronous copy
 * read by the pre-hydration script in layout.tsx to avoid a wrong-theme
 * flash; the Tauri store (`preferences.json`, key `theme_preference`) is
 * the canonical copy, matching how other preferences persist
 * (see PreferenceSettings.tsx).
 */

import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
} from 'react';

export type ThemePreference = 'system' | 'light' | 'dark';

export const THEME_STORAGE_KEY = 'meetily-theme';
const STORE_KEY = 'theme_preference';

function isThemePreference(value: unknown): value is ThemePreference {
  return value === 'system' || value === 'light' || value === 'dark';
}

interface ThemeContextValue {
  /** The user's stored preference (may be 'system') */
  preference: ThemePreference;
  /** The theme actually in effect right now */
  resolved: 'light' | 'dark';
  setPreference: (preference: ThemePreference) => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [preference, setPreferenceState] = useState<ThemePreference>(() => {
    if (typeof window === 'undefined') return 'system';
    try {
      const saved = window.localStorage.getItem(THEME_STORAGE_KEY);
      return isThemePreference(saved) ? saved : 'system';
    } catch {
      return 'system';
    }
  });
  const [systemDark, setSystemDark] = useState<boolean>(() => {
    if (typeof window === 'undefined') return false;
    return window.matchMedia('(prefers-color-scheme: dark)').matches;
  });

  // Reconcile with the canonical Tauri store once it is reachable.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const { Store } = await import('@tauri-apps/plugin-store');
        const store = await Store.load('preferences.json');
        const saved = await store.get<string>(STORE_KEY);
        if (!cancelled && isThemePreference(saved)) {
          setPreferenceState(saved);
          try {
            window.localStorage.setItem(THEME_STORAGE_KEY, saved);
          } catch {
            /* localStorage unavailable; non-fatal */
          }
        }
      } catch {
        // Store plugin not reachable (e.g. plain browser dev); localStorage rules.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Track OS appearance for the 'system' preference.
  useEffect(() => {
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    mq.addEventListener('change', onChange);
    setSystemDark(mq.matches);
    return () => mq.removeEventListener('change', onChange);
  }, []);

  const resolved: 'light' | 'dark' =
    preference === 'dark' || (preference === 'system' && systemDark)
      ? 'dark'
      : 'light';

  // Apply the class that drives every Ledger CSS token.
  //
  // Native window chrome: tauri.conf.json no longer forces "Light", so the
  // titlebar follows the OS. Forcing light/dark in-app does NOT retint the
  // titlebar — that would need window.setTheme(), whose capability
  // (core:window:allow-set-theme) is deliberately not granted; the
  // tauri-security-contract test pins the permission list.
  useEffect(() => {
    document.documentElement.classList.toggle('dark', resolved === 'dark');
  }, [resolved]);

  const setPreference = useCallback((next: ThemePreference) => {
    setPreferenceState(next);
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, next);
    } catch {
      /* non-fatal */
    }
    (async () => {
      try {
        const { Store } = await import('@tauri-apps/plugin-store');
        const store = await Store.load('preferences.json');
        await store.set(STORE_KEY, next);
        await store.save();
      } catch (error) {
        console.error('[Theme] Failed to persist theme preference:', error);
      }
    })();
  }, []);

  return (
    <ThemeContext.Provider value={{ preference, resolved, setPreference }}>
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) {
    throw new Error('useTheme must be used within a ThemeProvider');
  }
  return ctx;
}
