// Translations. Every language is one JSON file in ./locales (`en.json`, `fr.json`...): to add a language, add a file
// named after its code (`de.json`) with the same keys and a `_name` (how the language writes its own name); it shows
// up in the language menu with no other change. A missing key falls back to English, then to the key itself, so an
// unfinished translation never breaks the panel (the i18n test lists what is missing).

import { createContext, useCallback, useContext, useMemo, useState, type ReactNode } from 'react';
import { pickLanguage, sortedLanguages, translate, type Dictionaries } from './core.ts';

const files = import.meta.glob('./locales/*.json', { eager: true, import: 'default' }) as Dictionaries;

/** Language code -> dictionary, from the file names. */
export const DICTIONARIES: Dictionaries = Object.fromEntries(Object.entries(files).map(([path, dict]) => [path.replace(/^.*\/(.+)\.json$/, '$1'), dict]));

/** Codes of the available languages, English first. */
export const LANGUAGES = sortedLanguages(Object.keys(DICTIONARIES));

/** How a language writes its own name. */
export const languageName = (code: string): string => DICTIONARIES[code]?._name ?? code;

const STORAGE_KEY = 'omu-language';

function initialLanguage(): string {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved && DICTIONARIES[saved]) return saved;
  } catch {
    /* storage may be blocked */
  }
  return pickLanguage(typeof navigator === 'undefined' ? [] : navigator.languages ?? [navigator.language], LANGUAGES);
}

// `<html lang>` is right from the first paint (a screen reader reads with the voice of the language), not only after a change
if (typeof document !== 'undefined') document.documentElement.lang = initialLanguage();

type Translate = (key: string, vars?: Record<string, string | number>) => string;
type Context = { language: string; setLanguage: (code: string) => void; t: Translate };
const I18n = createContext<Context | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [language, setLang] = useState(initialLanguage);
  const setLanguage = useCallback((code: string) => {
    setLang(code);
    document.documentElement.lang = code;
    try {
      localStorage.setItem(STORAGE_KEY, code);
    } catch {
      /* ignore */
    }
  }, []);
  const value = useMemo<Context>(() => ({ language, setLanguage, t: (key, vars) => translate(DICTIONARIES, language, key, vars) }), [language, setLanguage]);
  return <I18n.Provider value={value}>{children}</I18n.Provider>;
}

export function useI18n(): Context {
  const ctx = useContext(I18n);
  if (!ctx) throw new Error('useI18n outside I18nProvider');
  return ctx;
}

export const useT = (): Translate => useI18n().t;
