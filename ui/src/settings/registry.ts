/**
 * The settings of the program and of the extensions, declared and never written into the page by hand. A category is a page of
 * the list on the left of the settings (a category with a `parent` is a sub-category of it); a setting is a row of that page, in a
 * titled group of rows, with a control chosen by its `control.type`. The page (`SettingsPage.tsx`) is drawn from what is registered
 * here, and so are the command palette's settings: something registered while the page is open (by an extension) appears at once.
 */

import type { IconName } from '../components/Icon.tsx';
import { rank } from '../fuzzy.ts';

export type SettingValue = string | number | boolean;
export type Translate = (key: string, vars?: Record<string, string | number>) => string;
/** A value of a choice: its name, and what it means (the documentation of the setting tells it). */
export type Choice = { value: string; label: string; hint?: string };

/** How a setting is shown and changed: one control per type, found by name (see `controls.tsx`). */
export type SettingControl =
  | { type: 'toggle' }
  | { type: 'choice'; options: Choice[] | ((t: Translate) => Choice[]) }
  | { type: 'text'; placeholder?: string }
  | { type: 'number'; min?: number; max?: number; step?: number; whole?: boolean; /** What it is counted in: a translation key, or the text itself when `plain`. */ unit?: string }
  | { type: 'slider'; min: number; max: number; step?: number }
  /** A whole block of the page that a component draws (found by name in `sections.tsx`). */
  | { type: 'section'; component: string }
  /** What an extension draws itself in the space of the setting. May return what takes it away. */
  | { type: 'custom'; mount: (container: HTMLElement, api: { value: SettingValue; set: (value: SettingValue) => void }) => void | (() => void) };

/** Where the value of a setting lives. */
export type Binding =
  /** One of the settings the daemon saves (`Settings`). */
  | { source: 'daemon'; key: string }
  /** What the page keeps for itself (the language, the theme). */
  | { source: 'ui'; key: 'language' | 'theme' }
  /** A setting of an extension, kept in the browser. */
  | { source: 'extension'; extension: string; key: string; default: SettingValue }
  /** Nothing to keep: the control is a section that does its own saving. */
  | { source: 'none' };

export type SettingCategory = {
  id: string;
  /** A translation key, or the text itself when `plain` (what extensions give). */
  title: string;
  /** A line under the title of the page. */
  description?: string;
  /** The documentation of the page, in markdown (a translation key, or the text itself when `plain`): the information window of the page. */
  doc?: string;
  /** A small picture in front of a heading of the list. */
  icon?: IconName;
  plain?: boolean;
  /** The category this one is a sub-category of. A category that has sub-categories is a heading of the list: it opens its first one. */
  parent?: string;
  order: number;
};

export type SettingDecl = {
  id: string;
  category: string;
  title: string;
  /** What it does, in a line or two: shown under the title. */
  description?: string;
  /** What the information window says, in markdown (a translation key, or the text itself when `plain`). */
  doc?: string;
  /** Its control is the whole page: the title, the line and the documentation of the page are the ones that count, so the page shows the control alone. */
  bare?: boolean;
  /** The settings that go with it: the information window links to them. */
  related?: string[];
  /** It is shown only while another setting has this value (the addresses of the other devices only make sense once they are allowed). */
  when?: { setting: string; equals: SettingValue };
  plain?: boolean;
  /** The title of the group of rows it is in (the rows of one group are drawn together); none for a group without a title. */
  group?: string;
  /** Other words that find it in a search (a translation key, or the text itself when `plain`; a key with no translation counts for nothing). */
  keywords?: string;
  control: SettingControl;
  bind: Binding;
  order?: number;
};

/** Everything that is registered: the categories (the pages) and the settings. */
export type Snapshot = { categories: SettingCategory[]; settings: SettingDecl[] };

let snapshot: Snapshot = { categories: [], settings: [] };
const listeners = new Set<() => void>();
const emit = () => listeners.forEach((l) => l());

const withEntry = <T extends { id: string }>(list: T[], entry: T): T[] => [...list.filter((e) => e.id !== entry.id), entry];

/** Adds a category (or replaces the one with the same id). Returns what takes it away, with its settings and its sub-categories. */
export function registerCategory(category: SettingCategory): () => void {
  snapshot = { ...snapshot, categories: withEntry(snapshot.categories, category) };
  emit();
  return () => {
    const gone = new Set([category.id, ...snapshot.categories.filter((c) => c.parent === category.id).map((c) => c.id)]);
    snapshot = { categories: snapshot.categories.filter((c) => c !== category && !gone.has(c.id)), settings: snapshot.settings.filter((s) => !gone.has(s.category)) };
    emit();
  };
}

/** Adds a setting (or replaces the one with the same id). Returns what takes it away. */
export function registerSetting(decl: SettingDecl): () => void {
  snapshot = { ...snapshot, settings: withEntry(snapshot.settings, decl) };
  emit();
  return () => {
    snapshot = { ...snapshot, settings: snapshot.settings.filter((s) => s !== decl) };
    emit();
  };
}

export const subscribe = (listener: () => void): (() => void) => {
  listeners.add(listener);
  return () => listeners.delete(listener);
};

export const getSnapshot = (): Snapshot => snapshot;

/** Empties the store (tests). */
export function resetSettings(): void {
  snapshot = { categories: [], settings: [] };
  emit();
}

/** The text of a title or a description: translated, unless it was given as it is. */
export const textOf = (t: Translate, plain: boolean | undefined, text: string): string => (plain ? text : t(text));

/** The other words that find a setting, in the language of the person (nothing when the key has no translation: its name is not a word). */
export function keywordsOf(decl: SettingDecl, t: Translate): string {
  if (!decl.keywords) return '';
  const words = textOf(t, decl.plain, decl.keywords);
  return words === decl.keywords && !decl.plain ? '' : words;
}

const byOrder = (t: Translate) => (a: SettingCategory, b: SettingCategory) => a.order - b.order || textOf(t, a.plain, a.title).localeCompare(textOf(t, b.plain, b.title));

/** The categories that are not inside another, in the order they are shown. */
export function topCategories(state: Snapshot, t: Translate): SettingCategory[] {
  return state.categories.filter((c) => !c.parent || !state.categories.some((p) => p.id === c.parent)).sort(byOrder(t));
}

/** The sub-categories of a category, in the order they are shown. */
export function subCategories(state: Snapshot, parent: string, t: Translate): SettingCategory[] {
  return state.categories.filter((c) => c.parent === parent).sort(byOrder(t));
}

/** The page a category leads to: itself, or its first sub-category when it is a heading. */
export function pageOf(state: Snapshot, id: string, t: Translate): SettingCategory | undefined {
  const category = state.categories.find((c) => c.id === id);
  if (!category) return undefined;
  return subCategories(state, id, t)[0] ?? category;
}

/** Whether a setting is shown: it has no condition, or the setting it depends on has the value it waits for. */
export const isShown = (decl: SettingDecl, valueOf: (setting: string) => SettingValue | undefined): boolean => !decl.when || valueOf(decl.when.setting) === decl.when.equals;

/** The settings of a category, in the order they are shown (the order they were declared in, unless they ask for another). */
export function settingsIn(state: Snapshot, category: string): SettingDecl[] {
  return state.settings
    .map((s, at) => ({ s, at }))
    .filter(({ s }) => s.category === category)
    .sort((a, b) => (a.s.order ?? 0) - (b.s.order ?? 0) || a.at - b.at)
    .map(({ s }) => s);
}

/** The settings of a page cut into the groups they were declared in, in the order the groups first appear. */
export function groupsOf(settings: SettingDecl[]): { title?: { text: string; plain?: boolean }; items: SettingDecl[] }[] {
  const groups: { key: string; title?: { text: string; plain?: boolean }; items: SettingDecl[] }[] = [];
  for (const s of settings) {
    const key = s.group ?? '';
    let group = groups.find((g) => g.key === key);
    if (!group) {
      group = { key, title: s.group ? { text: s.group, plain: s.plain } : undefined, items: [] };
      groups.push(group);
    }
    group.items.push(s);
  }
  return groups;
}

/** Without capitals and accents, so that "reseau" finds "Réseau". */
const plainText = (s: string) => s.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();

/**
 * The settings that a search finds, the best first: it looks at the title, the description, the extra words and the category. Every
 * word typed must be in the text; only when no setting has them all are the letters allowed to be apart (a typo, an abbreviation).
 */
export function searchSettings(state: Snapshot, query: string, t: Translate, shown: (decl: SettingDecl) => boolean = () => true): SettingDecl[] {
  const category = (id: string) => state.categories.find((c) => c.id === id);
  const words = (s: SettingDecl) => {
    const c = category(s.category);
    return [textOf(t, s.plain, s.title), s.description ? textOf(t, s.plain, s.description) : '', keywordsOf(s, t), c ? textOf(t, c.plain, c.title) : ''].join(' ');
  };
  const wanted = plainText(query).split(/\s+/).filter(Boolean);
  const candidates = state.settings.filter(shown);
  const exact = candidates.filter((s) => {
    const text = plainText(words(s));
    return wanted.every((w) => text.includes(w));
  });
  if (exact.length > 0) {
    // a title that has the words comes before a description that has them
    const inTitle = (s: SettingDecl) => wanted.every((w) => plainText(textOf(t, s.plain, s.title)).includes(w));
    return exact.map((s, at) => ({ s, at })).sort((a, b) => Number(inTitle(b.s)) - Number(inTitle(a.s)) || a.at - b.at).map(({ s }) => s);
  }
  return rank(query, candidates, words).map((r) => r.item);
}
