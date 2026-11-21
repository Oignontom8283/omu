import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { FALLBACK, pickLanguage, placeholders, sortedLanguages, translate, type Dictionaries } from './core.ts';
import { FORMULA_FUNCTIONS, TEXT_METHODS, TEXT_SIZE } from '../formula.ts';
import { KINDS_BY_CATEGORY, PORTS, CATEGORIES, portsOf } from '../graph.ts';

const dir = join(import.meta.dirname, 'locales');
const dictionaries: Dictionaries = Object.fromEntries(
  readdirSync(dir)
    .filter((f) => f.endsWith('.json'))
    .map((f) => [f.replace(/\.json$/, ''), JSON.parse(readFileSync(join(dir, f), 'utf8'))]),
);
const english = dictionaries[FALLBACK];

test('English and French exist, and every language has a name', () => {
  assert.ok(english && dictionaries.fr);
  for (const [code, d] of Object.entries(dictionaries)) assert.ok(d._name, `${code} needs _name`);
});

test('every language has exactly the keys of English (add the missing ones, remove the stale ones)', () => {
  for (const [code, d] of Object.entries(dictionaries)) {
    const missing = Object.keys(english).filter((k) => !(k in d));
    const extra = Object.keys(d).filter((k) => !(k in english));
    assert.deepEqual({ code, missing, extra }, { code, missing: [], extra: [] });
  }
});

test('translations keep the placeholders of the English text', () => {
  for (const [code, d] of Object.entries(dictionaries)) {
    for (const [key, text] of Object.entries(d)) {
      assert.deepEqual(placeholders(text), placeholders(english[key] ?? ''), `${code}: ${key}`);
    }
  }
});

test('nothing is left empty', () => {
  for (const [code, d] of Object.entries(dictionaries)) for (const [k, v] of Object.entries(d)) assert.ok(v.trim().length > 0, `${code}: ${k}`);
});

test('every block, family, port and flow the editor shows has a text', () => {
  const needed: string[] = [];
  for (const c of CATEGORIES) {
    needed.push(`category.${c}`, `category.${c}.hint`);
    for (const t of KINDS_BY_CATEGORY[c]) needed.push(`kind.${t}`, `kind.${t}.hint`);
  }
  // the ports of every block, and of the blocks that branch with several conditions
  const shown = [...Object.values(PORTS), portsOf({ type: 'branch', branches: 3 }), portsOf({ type: 'select', branches: 3 })];
  for (const ports of shown) for (const port of ports) needed.push(port.label, `flow.${port.flow}`);
  for (const key of needed) assert.ok(key in english, `missing in English: ${key}`);
});

test('every function of an expression and of a test on texts is explained, and every block with a long documentation has its examples', () => {
  for (const f of FORMULA_FUNCTIONS) assert.ok(`formula.fn.${f.name}` in english, f.name);
  for (const f of [...TEXT_METHODS, TEXT_SIZE]) assert.ok(`test.fn.${f.name}` in english, f.name);
  for (const kind of ['math_expr', 'cond_expr', 'text_expr']) assert.ok(`kind.${kind}.examples` in english, kind);
});

test('lookup falls back to English, then to the key, and fills placeholders', () => {
  const dicts: Dictionaries = { en: { a: 'A', n: '{count} things' }, fr: { a: 'Á' } };
  assert.equal(translate(dicts, 'fr', 'a'), 'Á');
  assert.equal(translate(dicts, 'fr', 'n', { count: 3 }), '3 things', 'missing in French: English');
  assert.equal(translate(dicts, 'fr', 'zzz'), 'zzz', 'missing everywhere: the key');
  assert.equal(translate(dicts, 'de', 'a'), 'A', 'unknown language: English');
  assert.equal(translate(dicts, 'en', 'n', {}), '{count} things', 'unknown placeholder stays visible');
});

test('the language follows the browser preferences', () => {
  const available = ['en', 'fr', 'pt-BR'];
  assert.equal(pickLanguage(['fr-CA', 'en'], available), 'fr');
  assert.equal(pickLanguage(['de', 'fr'], available), 'fr', 'first preference that exists');
  assert.equal(pickLanguage(['pt-br'], available), 'pt-BR', 'case-insensitive exact match');
  assert.equal(pickLanguage(['ja'], available), 'en');
  assert.equal(pickLanguage([], available), 'en');
  assert.deepEqual(sortedLanguages(['fr', 'en', 'de']), ['en', 'de', 'fr']);
});
