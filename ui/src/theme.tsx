// Light and dark themes. The choice (system, light or dark) is remembered; "system" follows the operating system and
// changes with it. The resolved theme is written on the page (`<html data-theme="light|dark">`), which is what the
// stylesheet reads; the node editor is told the same through `colorMode`.

import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

export type ThemeChoice = 'system' | 'light' | 'dark';
export type ResolvedTheme = 'light' | 'dark';

const STORAGE_KEY = 'omu-theme';

export function resolveTheme(choice: ThemeChoice, systemDark: boolean): ResolvedTheme {
  return choice === 'system' ? (systemDark ? 'dark' : 'light') : choice;
}

function initialChoice(): ThemeChoice {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === 'light' || saved === 'dark' || saved === 'system') return saved;
  } catch {
    /* storage may be blocked */
  }
  return 'system';
}

type Context = { choice: ThemeChoice; setChoice: (c: ThemeChoice) => void; resolved: ResolvedTheme };
const Theme = createContext<Context | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [choice, setChoiceState] = useState<ThemeChoice>(initialChoice);
  const [systemDark, setSystemDark] = useState(() => window.matchMedia('(prefers-color-scheme: dark)').matches);

  useEffect(() => {
    const query = window.matchMedia('(prefers-color-scheme: dark)');
    const listener = (e: MediaQueryListEvent) => setSystemDark(e.matches);
    query.addEventListener('change', listener);
    return () => query.removeEventListener('change', listener);
  }, []);

  const resolved = resolveTheme(choice, systemDark);
  useEffect(() => {
    document.documentElement.dataset.theme = resolved;
  }, [resolved]);

  const value = useMemo<Context>(
    () => ({
      choice,
      resolved,
      setChoice: (c) => {
        setChoiceState(c);
        try {
          localStorage.setItem(STORAGE_KEY, c);
        } catch {
          /* ignore */
        }
      },
    }),
    [choice, resolved],
  );
  return <Theme.Provider value={value}>{children}</Theme.Provider>;
}

export function useTheme(): Context {
  const ctx = useContext(Theme);
  if (!ctx) throw new Error('useTheme outside ThemeProvider');
  return ctx;
}
