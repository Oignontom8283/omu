// Light and dark themes. The choice (system, light or dark) is remembered; "system" follows the operating system and
// changes with it. The resolved theme is written on the page (`<html data-theme="light|dark">`), which is what the
// stylesheet reads; the node editor is told the same through `colorMode`.

import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

export type ThemeChoice = 'system' | 'light' | 'dark' | 'contrast';
export type ResolvedTheme = 'light' | 'dark';

const STORAGE_KEY = 'omu-theme';

export function resolveTheme(choice: ThemeChoice, systemDark: boolean): ResolvedTheme {
  if (choice === 'contrast') return 'dark';
  return choice === 'system' ? (systemDark ? 'dark' : 'light') : choice;
}

function initialChoice(): ThemeChoice {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === 'light' || saved === 'dark' || saved === 'system' || saved === 'contrast') return saved;
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
    // the high contrast theme is the dark one with its colours pushed to the extremes
    if (choice === 'contrast') document.documentElement.dataset.contrast = 'true';
    else delete document.documentElement.dataset.contrast;
  }, [resolved, choice]);

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

/** Colours the person chose for themselves: a small JSON object of CSS variables (`{ "--primary": "#ff00aa" }`), kept in this browser. */
const VARS_KEY = 'omu-theme-vars';

/** The colours a person can change one by one: the CSS variable and the translation key of its name (all the others are reached through the text). */
export const COLOUR_TOKENS: { name: string; label: string }[] = [
  { name: '--primary', label: 'colour.primary' },
  { name: '--on-primary', label: 'colour.onPrimary' },
  { name: '--bg', label: 'colour.bg' },
  { name: '--panel', label: 'colour.panel' },
  { name: '--field', label: 'colour.field' },
  { name: '--text', label: 'colour.text' },
  { name: '--muted', label: 'colour.muted' },
  { name: '--line', label: 'colour.line' },
  { name: '--ok', label: 'colour.ok' },
  { name: '--warn', label: 'colour.warn' },
  { name: '--bad', label: 'colour.bad' },
];

/** The text of the chosen colours, or an empty object. */
export function loadThemeVars(): string {
  try {
    return localStorage.getItem(VARS_KEY) ?? '{}';
  } catch {
    return '{}';
  }
}

/** Puts the chosen colours on the page. Returns what was wrong with the text, or null. */
export function applyThemeVars(text: string): string | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text || '{}');
  } catch (e) {
    return e instanceof Error ? e.message : String(e);
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return 'not an object';
  const root = document.documentElement;
  for (const name of Object.keys(root.dataset.themeVars ? JSON.parse(root.dataset.themeVars) : {})) root.style.removeProperty(name);
  const given: Record<string, string> = {};
  for (const [name, value] of Object.entries(parsed as Record<string, unknown>)) {
    if (!/^--[a-z0-9-]+$/i.test(name) || typeof value !== 'string') return `${name}: a CSS variable (--name) and a text are expected`;
    root.style.setProperty(name, value);
    given[name] = value;
  }
  root.dataset.themeVars = JSON.stringify(given);
  try {
    localStorage.setItem(VARS_KEY, text);
  } catch {
    /* ignore */
  }
  return null;
}

export function useTheme(): Context {
  const ctx = useContext(Theme);
  if (!ctx) throw new Error('useTheme outside ThemeProvider');
  return ctx;
}
