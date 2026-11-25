import assert from 'node:assert/strict';
import { test } from 'node:test';
import { parseInline, parseMarkdown } from './richtext.ts';

test('a link to a setting names it by its id, which may hold dots and colons (the settings of an extension)', () => {
  assert.deepEqual(parseInline('See [Volume](setting:extension:fancy:volume) and [Language](setting:language).'), [
    { text: 'See ' },
    { text: 'Volume', link: { scheme: 'setting', id: 'extension:fancy:volume' } },
    { text: ' and ' },
    { text: 'Language', link: { scheme: 'setting', id: 'language' } },
    { text: '.' },
  ]);
});

test('plain text is a single piece', () => {
  assert.deepEqual(parseInline('Louder or quieter'), [{ text: 'Louder or quieter' }]);
  assert.deepEqual(parseInline(''), []);
});

test('links to objects name their kind and their target, the text around them is kept', () => {
  assert.deepEqual(parseInline('Adds [sound in 1](port:in) and [Gain](kind:gain), a [Sound](type:audio).'), [
    { text: 'Adds ' },
    { text: 'sound in 1', link: { scheme: 'port', id: 'in' } },
    { text: ' and ' },
    { text: 'Gain', link: { scheme: 'kind', id: 'gain' } },
    { text: ', a ' },
    { text: 'Sound', link: { scheme: 'type', id: 'audio' } },
    { text: '.' },
  ]);
});

test('code and bold are marked, brackets that are not links stay as they are', () => {
  assert.deepEqual(parseInline('its name (`firefox.exe`) is **required**'), [{ text: 'its name (' }, { text: 'firefox.exe', code: true }, { text: ') is ' }, { text: 'required', bold: true }]);
  assert.deepEqual(parseInline('a [b] (c) [d](e)'), [{ text: 'a [b] (c) [d](e)' }]);
});

test('a document is cut into headings, paragraphs, lists, tables and quotes', () => {
  const blocks = parseMarkdown(['# Gain', 'Louder', 'or quieter.', '', '## Inputs', '| Name | Type |', '|---|---|', '| [in](port:in) | [Sound](type:audio) |', '', '- one', '- two', '', '## Warning', '> careful'].join('\n'));
  assert.deepEqual(
    blocks.map((b) => b.type),
    ['heading', 'paragraph', 'heading', 'table', 'list', 'heading', 'quote'],
  );
  assert.deepEqual(blocks[1], { type: 'paragraph', inline: [{ text: 'Louder or quieter.' }] });
  const table = blocks[3];
  assert.ok(table.type === 'table');
  assert.equal(table.head.length, 2);
  assert.equal(table.rows.length, 1);
  assert.deepEqual(table.rows[0][0], [{ text: 'in', link: { scheme: 'port', id: 'in' } }]);
  const list = blocks[4];
  assert.ok(list.type === 'list' && list.items.length === 2);
});
