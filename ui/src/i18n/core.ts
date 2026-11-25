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

/**
 * The form of a text that goes with a count: `key.one`, `key.other`... (the categories of the language, from `Intl.PluralRules`), when the
 * dictionary has them, else `key` itself. A text with a count is therefore written once per form, never as "sound(s)".
 */
function formOf(dictionaries: Dictionaries, language: string, key: string, count: number): string {
  let category = 'other';
  try {
    category = new Intl.PluralRules(language).select(count);
  } catch {
    /* an unknown language code: the general form */
  }
  for (const form of [category, 'other']) {
    const plural = `${key}.${form}`;
    if (dictionaries[language]?.[plural] !== undefined || dictionaries[FALLBACK]?.[plural] !== undefined) return plural;
  }
  return key;
}

/** Looks `key` up in a language, then in English, then returns the key itself; fills `{name}` placeholders (a `count` picks the plural form). */
export function translate(dictionaries: Dictionaries, language: string, key: string, vars?: Record<string, string | number>): string {
  const wanted = typeof vars?.count === 'number' ? formOf(dictionaries, language, key, vars.count) : key;
  const text = dictionaries[language]?.[wanted] ?? dictionaries[FALLBACK]?.[wanted] ?? key;
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

/** The language the page is written in now (the one of `<html lang>`), English when there is no page. */
export const currentLanguage = (): string => (typeof document === 'undefined' ? '' : document.documentElement.lang) || FALLBACK;

/** A date and time the way the language writes them. */
export function formatDateTime(date: Date | number | string, language: string = currentLanguage()): string {
  try {
    return new Intl.DateTimeFormat(language, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(date));
  } catch {
    return new Date(date).toISOString();
  }
}

/** A size in bytes with the unit of the language (`850 KB`, `12,4 Mo`). */
export function formatByteSize(bytes: number, language: string = currentLanguage()): string {
  const units = ['byte', 'kilobyte', 'megabyte', 'gigabyte', 'terabyte'];
  let value = bytes;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i++;
  }
  try {
    return new Intl.NumberFormat(language, { style: 'unit', unit: units[i], unitDisplay: 'short', maximumFractionDigits: i === 0 || value >= 100 ? 0 : 1 }).format(value);
  } catch {
    return `${Math.round(value)} ${units[i]}`;
  }
}
