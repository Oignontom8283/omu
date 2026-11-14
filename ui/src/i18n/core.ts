// The pure part of the translations (no React, no bundler): picking a language and looking a key up.

export type Dictionary = Record<string, string>;
export type Dictionaries = Record<string, Dictionary>;

export const FALLBACK = 'en';

/** The best available language for a browser's list of preferences (`fr-CA` -> `fr`); English when none fits. */
export function pickLanguage(preferred: readonly string[], available: readonly string[]): string {
  for (const p of preferred) {
    const lower = p.toLowerCase();
    const exact = available.find((a) => a.toLowerCase() === lower);
    if (exact) return exact;
    const base = available.find((a) => a.toLowerCase() === lower.split('-')[0]);
    if (base) return base;
  }
  return FALLBACK;
}

/** Looks `key` up in a language, then in English, then returns the key itself; fills `{name}` placeholders. */
export function translate(dictionaries: Dictionaries, language: string, key: string, vars?: Record<string, string | number>): string {
  const text = dictionaries[language]?.[key] ?? dictionaries[FALLBACK]?.[key] ?? key;
  return vars ? text.replace(/\{(\w+)\}/g, (whole, name: string) => (name in vars ? String(vars[name]) : whole)) : text;
}

/** Codes sorted with English first. */
export function sortedLanguages(codes: readonly string[]): string[] {
  return [...codes].sort((a, b) => (a === FALLBACK ? -1 : b === FALLBACK ? 1 : a.localeCompare(b)));
}

/** The placeholders (`{count}`) a text uses, sorted. */
export function placeholders(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
}
