import assert from 'node:assert/strict';
import { test } from 'node:test';
import { MODULE_NEEDED, moduleStatus } from './modules.ts';
import type { ModuleInfo } from './types.ts';

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
