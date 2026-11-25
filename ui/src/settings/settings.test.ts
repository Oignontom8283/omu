import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { beforeEach, test } from 'node:test';
import { BUILTIN_CATEGORIES, builtinSettings } from './builtin.ts';
import { categoryDocument, settingDocument } from './doc.ts';
import { extensionCategory, getSetting, onSettingChange, registerExtension, registerExtensionSetting } from './extensionApi.ts';
import { resetExtensionValues, writeExtensionSetting } from './extensionValues.ts';
import { getSnapshot, groupsOf, isShown, pageOf, registerCategory, registerSetting, resetSettings, searchSettings, settingsIn, subCategories, textOf, topCategories, type SettingCategory, type SettingDecl, type Translate } from './registry.ts';

// a translation that shows the key it was given, so that a test sees what was asked for
const t: Translate = (key) => `<${key}>`;
const toggle = (id: string, category: string, extra: Partial<SettingDecl> = {}): SettingDecl => ({ id, category, title: id, control: { type: 'toggle' }, bind: { source: 'none' }, ...extra });

beforeEach(() => {
  resetSettings();
  resetExtensionValues();
});

test('the categories are a tree: headings with their pages, in their order, and a category of the same id is replaced', () => {
  registerCategory({ id: 'b', title: 'B', order: 20 });
  registerCategory({ id: 'a', title: 'A', order: 10 });
  registerCategory({ id: 'a2', title: 'A2', parent: 'a', order: 2 });
  registerCategory({ id: 'a1', title: 'A1', parent: 'a', order: 1 });
  assert.deepEqual(topCategories(getSnapshot(), t).map((c) => c.id), ['a', 'b']);
  assert.deepEqual(subCategories(getSnapshot(), 'a', t).map((c) => c.id), ['a1', 'a2']);
  assert.equal(pageOf(getSnapshot(), 'a', t)?.id, 'a1', 'a heading opens its first page');
  assert.equal(pageOf(getSnapshot(), 'b', t)?.id, 'b', 'a category without pages is a page');
  assert.equal(pageOf(getSnapshot(), 'zz', t), undefined);
  registerCategory({ id: 'b', title: 'B again', order: 5 });
  assert.deepEqual(topCategories(getSnapshot(), t).map((c) => c.title), ['B again', 'A']);
});

test('taking a category away takes its sub-categories and settings with it, and a setting goes alone when it is asked to', () => {
  const stop = registerCategory({ id: 'a', title: 'A', order: 1 });
  registerCategory({ id: 'a1', title: 'A1', parent: 'a', order: 1 });
  registerCategory({ id: 'b', title: 'B', order: 2 });
  registerSetting(toggle('one', 'a1'));
  const stopTwo = registerSetting(toggle('two', 'b'));
  registerSetting(toggle('three', 'b'));
  stopTwo();
  assert.deepEqual(settingsIn(getSnapshot(), 'b').map((s) => s.id), ['three']);
  stop();
  assert.deepEqual(getSnapshot().categories.map((c) => c.id), ['b']);
  assert.deepEqual(getSnapshot().settings.map((s) => s.id), ['three']);
});

test('the settings of a category keep the order they were declared in, unless one asks for another, and are cut into their groups', () => {
  registerCategory({ id: 'a', title: 'A', order: 1 });
  registerSetting(toggle('second', 'a', { group: 'Look' }));
  registerSetting(toggle('first', 'a', { order: -1 }));
  registerSetting(toggle('third', 'a', { group: 'Look' }));
  registerSetting(toggle('fourth', 'a', { group: 'Feel' }));
  const settings = settingsIn(getSnapshot(), 'a');
  assert.deepEqual(settings.map((s) => s.id), ['first', 'second', 'third', 'fourth']);
  const groups = groupsOf(settings);
  assert.deepEqual(groups.map((g) => [g.title?.text, g.items.map((s) => s.id)]), [
    [undefined, ['first']],
    ['Look', ['second', 'third']],
    ['Feel', ['fourth']],
  ]);
});

test('a search finds a setting by its title, its description, its extra words and its category, and says things as they are written', () => {
  registerCategory({ id: 'a', title: 'settings.section.system', order: 1 });
  registerCategory({ id: 'ext', title: 'Fancy Tool', plain: true, order: 0 });
  registerSetting(toggle('tray', 'a', { title: 'settings.tray', description: 'settings.trayHint', keywords: 'notification icon' }));
  registerSetting(toggle('mine', 'ext', { title: 'Loudness', description: 'How loud it is', plain: true }));
  const ids = (q: string) => searchSettings(getSnapshot(), q, t).map((s) => s.id);
  assert.deepEqual(ids('tray'), ['tray'], 'its title');
  assert.deepEqual(ids('notification'), ['tray'], 'its extra words');
  assert.deepEqual(ids('loudness'), ['mine'], 'a title written as it is');
  assert.deepEqual(ids('how loud'), ['mine'], 'its description');
  assert.deepEqual(ids('fancy'), ['mine'], 'the name of its category');
  assert.deepEqual(ids('Réseau loudn'), [], 'every word must be there');
  assert.deepEqual(ids('lodness'), ['mine'], 'a typo is still found, by the letters');
  assert.deepEqual(ids('zzzz'), []);
  assert.equal(textOf(t, true, 'Loudness'), 'Loudness');
  assert.equal(textOf(t, false, 'settings.tray'), '<settings.tray>');
});

test('an extension has a category of its own, its settings are written as it gave them, and their values are kept for it', () => {
  registerExtension({ id: 'fancy', name: 'Fancy Tool', description: 'Does fancy things' });
  registerExtensionSetting({ extension: 'fancy', id: 'volume', title: 'Volume', control: { type: 'slider', min: 0, max: 10 }, default: 5 });
  registerExtensionSetting({ extension: 'fancy', id: 'name', title: 'Name', control: { type: 'text' }, default: 'x' });
  const category = getSnapshot().categories.find((c) => c.id === extensionCategory('fancy'));
  assert.deepEqual([category?.parent, category?.plain, category?.title], ['extensions', true, 'Fancy Tool']);
  const volume = settingsIn(getSnapshot(), extensionCategory('fancy'))[0];
  assert.deepEqual([volume.id, volume.plain], ['extension:fancy:volume', true]);
  assert.deepEqual(volume.bind, { source: 'extension', extension: 'fancy', key: 'volume', default: 5 });

  assert.equal(getSetting('fancy', 'volume', 5), 5, 'the default until it is changed');
  const seen: unknown[] = [];
  const stop = onSettingChange('fancy', 'volume', (v) => seen.push(v));
  writeExtensionSetting('fancy', 'volume', 8);
  assert.equal(getSetting('fancy', 'volume', 5), 8);
  stop();
  writeExtensionSetting('fancy', 'volume', 9);
  assert.deepEqual(seen, [8], 'told of a change until it stops listening');
});

test('an extension documents itself and its settings in the same window as the program does, and its settings may point to one another', () => {
  registerExtension({ id: 'fancy', name: 'Fancy Tool', description: 'Does fancy things', doc: 'It does **a lot**.' });
  registerExtensionSetting({ extension: 'fancy', id: 'volume', title: 'Volume', description: 'How loud.', doc: 'Louder is louder.', related: ['name'], control: { type: 'slider', min: 0, max: 10 }, default: 5 });
  registerExtensionSetting({ extension: 'fancy', id: 'name', title: 'Name', control: { type: 'text' }, default: 'x' });
  const state = getSnapshot();
  const category = state.categories.find((c) => c.id === extensionCategory('fancy')) as SettingCategory;
  assert.ok(categoryDocument(state, category, t, () => undefined).source.includes('It does **a lot**.'));
  const volume = state.settings.find((s) => s.id === 'extension:fancy:volume') as SettingDecl;
  const source = settingDocument(state, volume, t).source;
  assert.ok(source.includes('Louder is louder.') && source.includes('- [Name](setting:extension:fancy:name)'), 'related settings are named by their id inside the extension');
});

test('a value of another type than the setting has is not believed', () => {
  writeExtensionSetting('fancy', 'volume', 'loud');
  assert.equal(getSetting('fancy', 'volume', 5), 5, 'a text where a number is wanted: the default');
  assert.equal(getSetting('fancy', 'volume', 'quiet'), 'loud', 'the same value for a setting that is a text');
});

const locale = (code: string) => JSON.parse(readFileSync(new URL(`../i18n/locales/${code}.json`, import.meta.url), 'utf8')) as Record<string, string>;
const translator = (code: string): Translate => {
  const words = locale(code);
  return (key, vars) => (words[key] ?? key).replace(/\{(\w+)\}/g, (_, name: string) => String(vars?.[name] ?? ''));
};

test('the settings of the program: every one is in a page that exists, every section has a block, and every text exists in every language', () => {
  const settings = builtinSettings([{ value: 'en', label: 'English' }]);
  const categories = new Set(BUILTIN_CATEGORIES.map((c) => c.id));
  assert.equal(new Set(settings.map((s) => s.id)).size, settings.length, 'one id each');
  assert.equal(categories.size, BUILTIN_CATEGORIES.length);
  const sections = new Set(['remote', 'certificates', 'sounds', 'shortcuts', 'colours', 'extensions']);
  for (const code of ['en', 'fr']) {
    const words = locale(code);
    const t = translator(code);
    const has = (where: string, key: string | undefined) => assert.ok(key !== undefined && key in words, `${code}: ${where}: ${key}`);
    for (const s of settings) {
      assert.ok(categories.has(s.category), `${s.id}: its page`);
      assert.ok(!BUILTIN_CATEGORIES.some((c) => c.parent === s.category), `${s.id}: a heading has no settings of its own, its pages do`);
      if (s.group) has(`${s.id} group`, s.group);
      if (s.control.type === 'section') assert.ok(sections.has(s.control.component), `${s.id}: the block ${s.control.component}`);
      has(`${s.id} title`, s.title);
      has(`${s.id} description`, s.description);
      // a setting that is the block of its page is told about by the page; the others carry their own documentation
      if (!s.bare) has(`${s.id} documentation`, s.doc);
      if (s.control.type === 'choice' && typeof s.control.options === 'function') {
        for (const o of s.control.options(t)) {
          // the languages are names that say it all; every other choice says what each value does
          assert.ok(o.label && (s.id === 'language' || o.hint), `${code}: ${s.id}: the option ${o.value} has a name and a meaning`);
          assert.ok(!o.label.startsWith('setting.') && !(o.hint ?? '').startsWith('setting.'), `${code}: ${s.id}: the option ${o.value} is translated`);
        }
      }
      if (s.control.type === 'number' && s.control.unit) has(`${s.id} unit`, s.control.unit);
      for (const id of s.related ?? []) assert.ok(settings.some((x) => x.id === id), `${s.id}: related ${id} exists`);
      if (s.when) assert.ok(settings.some((x) => x.id === s.when?.setting), `${s.id}: the setting it waits for exists`);
    }
    for (const c of BUILTIN_CATEGORIES) {
      has(`${c.id} title`, c.title);
      if (c.description) has(`${c.id} description`, c.description);
      if (c.doc) has(`${c.id} documentation`, c.doc);
      assert.ok(!c.parent || categories.has(c.parent), `${c.id}: its parent`);
      // every page explains itself; a heading is only a name
      if (c.parent) assert.ok(c.description && c.doc, `${c.id}: a page has its line and its documentation`);
    }
  }
  // what the command palette has always found is still there
  for (const id of ['language', 'theme', 'tray', 'logon', 'trust', 'remote']) assert.ok(settings.some((s) => s.id === id), id);
});

test('a setting that depends on another is shown only while that one has the value it waits for, and a search does not find what is hidden', () => {
  registerCategory({ id: 'a', title: 'A', order: 1 });
  registerSetting(toggle('on', 'a', { title: 'Allowed' }));
  registerSetting(toggle('addresses', 'a', { title: 'Addresses', when: { setting: 'on', equals: true } }));
  const values: Record<string, boolean> = { on: false };
  const shown = (s: SettingDecl) => isShown(s, (id) => values[id]);
  assert.deepEqual(settingsIn(getSnapshot(), 'a').filter(shown).map((s) => s.id), ['on']);
  assert.deepEqual(searchSettings(getSnapshot(), 'addresses', t, shown), []);
  values.on = true;
  assert.deepEqual(settingsIn(getSnapshot(), 'a').filter(shown).map((s) => s.id), ['on', 'addresses']);
  assert.deepEqual(searchSettings(getSnapshot(), 'addresses', t, shown).map((s) => s.id), ['addresses']);
});

test('the keywords of a setting find it in the language they are written in, and a key nobody translated finds nothing', () => {
  registerCategory({ id: 'a', title: 'A', order: 1 });
  registerSetting(toggle('tray', 'a', { title: 'settings.tray', keywords: 'setting.tray.keywords' }));
  const words: Translate = (key) => (key === 'setting.tray.keywords' ? 'systray horloge' : key);
  assert.deepEqual(searchSettings(getSnapshot(), 'horloge', words).map((s) => s.id), ['tray']);
  assert.deepEqual(searchSettings(getSnapshot(), 'keywords', (key) => key).map((s) => s.id), [], 'the name of the missing key is not a word');
});

test('the documentation of a setting is made of what it declares: its line, its text, the meaning of each value, its range and what goes with it', () => {
  registerCategory({ id: 'a', title: 'Look', order: 1 });
  const choice: SettingDecl['control'] = { type: 'choice', options: [{ value: 'light', label: 'Light', hint: 'White.' }, { value: 'dark', label: 'Dark', hint: 'Black | night.' }] };
  registerSetting(toggle('theme', 'a', { title: 'Theme', description: 'Light or dark.', doc: 'The **colours** of the program.', plain: true, related: ['volume', 'gone'], control: choice }));
  registerSetting(toggle('volume', 'a', { title: 'Volume', plain: true, control: { type: 'number', min: 0, max: 10, unit: 'dB' } }));
  registerSetting(toggle('size', 'a', { title: 'Size', plain: true, control: { type: 'number', min: 16, unit: 'MB' } }));
  registerSetting(toggle('language', 'a', { title: 'Language', plain: true, control: { type: 'choice', options: [{ value: 'en', label: 'English' }] } }));
  const state = getSnapshot();
  const doc = (id: string) => settingDocument(state, state.settings.find((s) => s.id === id) as SettingDecl, t);
  assert.equal(doc('theme').title, 'Theme');
  const lines = doc('theme').source.split('\n');
  assert.equal(lines[0], '# Theme');
  assert.ok(lines.includes('Light or dark.') && lines.includes('The **colours** of the program.'));
  assert.ok(lines.includes('| **Dark** | Black / night. |'), 'a bar in a meaning does not cut the table');
  assert.ok(lines.includes('- [Volume](setting:volume)') && !doc('theme').source.includes('gone'), 'a link to a setting that exists, none to the others');
  assert.ok(doc('volume').source.includes('<settings.doc.rangeText>'), 'a range with both ends');
  assert.ok(doc('size').source.includes('<settings.doc.minText>'), 'a range with a smallest value only');
  assert.ok(!doc('language').source.includes('settings.doc.values'), 'a list of names that say it all needs no table');
  // the page lists its settings, each a link
  const page = categoryDocument(state, state.categories[0], t, () => undefined);
  assert.ok(page.source.includes('- [Theme](setting:theme)') && page.source.includes('settings.doc.inPage'));
});
