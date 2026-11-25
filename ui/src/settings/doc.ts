// The text of the information window of a setting or of a page of the settings. It is made of what the setting declares (its title, what it
// does, its documentation) and of what is known about its control (the values a choice offers and what each one means, the range of a number),
// so that a new setting is documented by declaring it and no list of values is written twice.

import { isShown, settingsIn, textOf, type SettingDecl, type SettingCategory, type SettingValue, type Snapshot, type Translate } from './registry.ts';

/** What an information window shows: a document in markdown (its first line is its title). */
export type InfoDoc = { title: string; source: string };

/** A table cell cannot hold a bar or a line break. */
const cell = (text: string) => text.replace(/\|/g, '/').replace(/\s*\n\s*/g, ' ');

/** The documentation of a setting. */
export function settingDocument(state: Snapshot, decl: SettingDecl, t: Translate): InfoDoc {
  const title = textOf(t, decl.plain, decl.title);
  const lines = [`# ${title}`, ''];
  if (decl.description) lines.push(textOf(t, decl.plain, decl.description), '');
  if (decl.doc) lines.push(textOf(t, decl.plain, decl.doc), '');
  const control = decl.control;
  if (control.type === 'choice') {
    const options = typeof control.options === 'function' ? control.options(t) : control.options;
    // a list of names that say it all (the languages) needs no table
    if (options.some((o) => o.hint)) {
      lines.push(`## ${t('settings.doc.values')}`, `| ${t('settings.doc.value')} | ${t('settings.doc.meaning')} |`, '|---|---|', ...options.map((o) => `| **${cell(o.label)}** | ${cell(o.hint ?? '')} |`), '');
    }
  } else if ((control.type === 'number' || control.type === 'slider') && control.min !== undefined) {
    const unit = control.type === 'number' && control.unit ? ` ${textOf(t, decl.plain, control.unit)}` : '';
    lines.push(`## ${t('settings.doc.range')}`, control.max !== undefined ? t('settings.doc.rangeText', { min: control.min, max: control.max, unit }) : t('settings.doc.minText', { min: control.min, unit }), '');
  }
  const related = (decl.related ?? []).map((id) => state.settings.find((s) => s.id === id)).filter((s): s is SettingDecl => Boolean(s));
  if (related.length > 0) lines.push(`## ${t('settings.doc.seeAlso')}`, ...related.map((s) => `- [${cell(textOf(t, s.plain, s.title))}](setting:${s.id})`), '');
  return { title, source: lines.join('\n') };
}

/** The documentation of a page: what it is for, then the settings that are in it (each one a link to its own documentation). */
export function categoryDocument(state: Snapshot, category: SettingCategory, t: Translate, valueOf: (setting: string) => SettingValue | undefined): InfoDoc {
  const title = textOf(t, category.plain, category.title);
  const lines = [`# ${title}`, ''];
  if (category.description) lines.push(textOf(t, category.plain, category.description), '');
  if (category.doc) lines.push(textOf(t, category.plain, category.doc), '');
  const inside = settingsIn(state, category.id).filter((s) => isShown(s, valueOf));
  if (inside.length > 0) lines.push(`## ${t('settings.doc.inPage')}`, ...inside.map((s) => `- [${cell(textOf(t, s.plain, s.title))}](setting:${s.id})`), '');
  return { title, source: lines.join('\n') };
}
