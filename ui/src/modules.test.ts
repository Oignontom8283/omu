import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { MODULE_NEEDED, moduleDocument, moduleStatus, moduleText } from './modules.ts';
import type { ModuleInfo } from './types.ts';

// the texts of a language, the way the program asks for them
const translator = (code: string) => {
  const words = JSON.parse(readFileSync(new URL(`./i18n/locales/${code}.json`, import.meta.url), 'utf8')) as Record<string, string>;
  return (key: string, vars?: Record<string, string | number>) => (words[key] ?? key).replace(/\{(\w+)\}/g, (_, name: string) => String(vars?.[name] ?? ''));
};

const module = (change: Partial<ModuleInfo>): ModuleInfo => ({
  id: 'ffmpeg',
  name: 'FFmpeg',
  description: '',
  enabled: false,
  available: false,
  downloadable: true,
  download_mb: 115,
  ...change,
});

test('a module is installing, on, ready or missing', () => {
  assert.equal(moduleStatus(module({})), 'missing');
  assert.equal(moduleStatus(module({ available: true })), 'ready');
  assert.equal(moduleStatus(module({ available: true, enabled: true })), 'on');
  assert.equal(moduleStatus(module({ installing: { done: 1, total: 10 } })), 'installing');
  // on in the settings but nothing there any more (the copy was deleted by hand): not on
  assert.equal(moduleStatus(module({ enabled: true })), 'missing');
});

test('the code of a request that needs a module matches the daemon', () => {
  assert.equal(MODULE_NEEDED, -32005);
});

test('a text of a module is the translation, or what the module says of itself when it was never translated', () => {
  const ffmpeg = { id: 'ffmpeg', name: 'FFmpeg (engine)', description: 'Reads files.', enabled: false, available: true, source: 'local' as const, version: '8', downloadable: true, download_mb: 100 };
  const words = translator('en');
  assert.equal(moduleText(ffmpeg, 'name', words), 'FFmpeg');
  assert.equal(moduleText({ ...ffmpeg, id: 'unknown' }, 'description', words), 'Reads files.');
  const doc = moduleDocument(ffmpeg, words);
  assert.ok(doc.startsWith('# FFmpeg\n') && doc.includes('## What it is for') && doc.includes('| Version | 8 |'));
  assert.ok(!moduleDocument({ ...ffmpeg, id: 'unknown', available: false }, words).includes('## What it is for'), 'no text of its own: only the details');
  assert.ok(moduleDocument({ ...ffmpeg, available: false }, words).includes('about 100 MB'), 'what a download weighs');
});
