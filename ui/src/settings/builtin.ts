// The settings of the program itself. Nothing about them is written into the page: it draws what is declared here, the categories as a tree
// on the left (a heading with its pages under it) and the settings of a page one under the other. Every text is a translation key made from
// the id (`setting.<id>` is the title, `.hint` the line under it, `.doc` the documentation of its information window, `.keywords` the other words that find it in a search, `.option.<value>` and
// `.option.<value>.hint` the values of a choice), so a setting is complete when its keys exist in every language (the tests check it).

import type { IconName } from '../components/Icon.tsx';
import { MIN_CACHE_MB } from '../soundboard.ts';
import type { Choice, SettingCategory, SettingControl, SettingDecl, Translate } from './registry.ts';

/** A heading of the tree: it has pages under it and nothing of its own. */
const heading = (id: string, order: number, icon: IconName): SettingCategory => ({ id, title: `settings.page.${id}`, icon, order });

/** A page of the settings, with what it is for and its documentation. */
const page = (id: string, parent: string, order: number): SettingCategory => ({
  id,
  title: `settings.page.${id}`,
  description: `settings.page.${id}.hint`,
  doc: `settings.page.${id}.doc`,
  parent,
  order,
});

export const BUILTIN_CATEGORIES: SettingCategory[] = [
  heading('general', 10, 'gear'),
  page('appearance', 'general', 10),
  page('system', 'general', 20),

  heading('security', 20, 'shield'),
  page('connections', 'security', 10),
  page('remote_access', 'security', 20),

  heading('sound', 30, 'speaker'),
  page('sounds', 'sound', 10),

  heading('keyboard', 40, 'keyboard'),
  page('shortcuts', 'keyboard', 10),

  heading('extensions', 50, 'stack'),
  page('installed', 'extensions', 0),
];

const none = { source: 'none' } as const;

/** A setting, with its texts taken from its id. */
const setting = (id: string, category: string, control: SettingControl, bind: SettingDecl['bind'], extra: Partial<SettingDecl> = {}): SettingDecl => ({
  id,
  category,
  title: `setting.${id}`,
  description: `setting.${id}.hint`,
  doc: `setting.${id}.doc`,
  keywords: `setting.${id}.keywords`,
  control,
  bind,
  ...extra,
});

/** The only block of a page: it takes the texts of the page (a search finds it by the name of the page). */
const pageBlock = (id: string, category: string, component: string): SettingDecl => ({
  id,
  category,
  title: `settings.page.${category}`,
  description: `settings.page.${category}.hint`,
  keywords: `setting.${id}.keywords`,
  control: { type: 'section', component },
  bind: none,
  bare: true,
});

/** The values of a choice: each has a name and what it means, both translated from the id of the setting and the value. */
const options =
  (id: string, values: string[]) =>
  (t: Translate): Choice[] =>
    values.map((value) => ({ value, label: t(`setting.${id}.option.${value}`), hint: t(`setting.${id}.option.${value}.hint`) }));

/** The settings of the program. The languages are the ones the program has (the list comes from its translation files). */
export const builtinSettings = (languages: Choice[]): SettingDecl[] => [
  setting('language', 'appearance', { type: 'choice', options: () => languages }, { source: 'ui', key: 'language' }, { related: ['theme'] }),
  setting('theme', 'appearance', { type: 'choice', options: options('theme', ['system', 'light', 'dark', 'contrast']) }, { source: 'ui', key: 'theme' }, { related: ['colours'] }),
  setting('colours', 'appearance', { type: 'section', component: 'colours' }, none, { related: ['theme'] }),

  setting('logon', 'system', { type: 'toggle' }, { source: 'daemon', key: 'start_at_logon' }, { related: ['tray'] }),
  setting('tray', 'system', { type: 'toggle' }, { source: 'daemon', key: 'show_tray_icon' }, { related: ['logon'] }),

  setting('trust', 'connections', { type: 'choice', options: options('trust', ['system', 'own']) }, { source: 'daemon', key: 'trust' }, { related: ['certificates_dir'] }),
  setting('certificates_dir', 'connections', { type: 'section', component: 'certificates' }, none, { related: ['trust'] }),

  setting('remote', 'remote_access', { type: 'toggle' }, { source: 'daemon', key: 'remote_access' }, { related: ['remote_addresses', 'trust'] }),
  setting('remote_addresses', 'remote_access', { type: 'section', component: 'remote' }, none, { related: ['remote'], when: { setting: 'remote', equals: true } }),


  // pages that are one block and nothing else: what is said about the page is what is said about the block
  pageBlock('sounds', 'sounds', 'sounds'),
  setting('sound_cache_mb', 'sounds', { type: 'number', min: MIN_CACHE_MB, step: 16, whole: true, unit: 'unit.mb' }, { source: 'daemon', key: 'sound_cache_mb' }, { related: ['compress_above_mb'] }),
  setting('compress_above_mb', 'sounds', { type: 'number', min: 0, step: 1, whole: true, unit: 'unit.mb' }, { source: 'daemon', key: 'compress_above_mb' }, { related: ['sound_cache_mb'] }),
  pageBlock('shortcuts', 'shortcuts', 'shortcuts'),
  pageBlock('extensions', 'installed', 'extensions'),
];
