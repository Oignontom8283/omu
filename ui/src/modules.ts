// What the modules panel works out, kept apart from the screen so it can be tested.

import type { ModuleInfo } from './types.ts';

/** The error code of a request that needs a module that is not switched on; the error's data is the module's id. */
export const MODULE_NEEDED = -32005;

/** Where a module stands: being downloaded, switched on and usable, usable but off, or not there. */
export function moduleStatus(m: ModuleInfo): 'installing' | 'on' | 'ready' | 'missing' {
  if (m.installing) return 'installing';
  if (m.enabled && m.available) return 'on';
  if (m.available) return 'ready';
  return 'missing';
}

type Translate = (key: string, vars?: Record<string, string | number>) => string;

/** A text of a module in the language of the program: its translation when there is one, what the module says about itself otherwise (a module that is not known yet). */
export function moduleText(m: ModuleInfo, field: 'name' | 'description', t: Translate): string {
  const key = `module.${m.id}.${field}`;
  const text = t(key);
  return text === key ? m[field] : text;
}

/** The documentation of a module: what it is for, what it takes to use it, and where it stands now. */
export function moduleDocument(m: ModuleInfo, t: Translate): string {
  const key = `module.${m.id}.doc`;
  const doc = t(key);
  const lines = [`# ${moduleText(m, 'name', t)}`, '', moduleText(m, 'description', t), ''];
  if (doc !== key) lines.push(doc, '');
  const rows = [[t('modules.doc.state'), t(`modules.state.${moduleStatus(m)}`)]];
  if (m.available) rows.push([t('modules.doc.source'), t(m.source === 'local' ? 'modules.fromLocal' : 'modules.fromSystem')]);
  if (m.version) rows.push([t('modules.doc.version'), m.version]);
  if (!m.available && m.downloadable) rows.push([t('modules.doc.download'), t('modules.doc.downloadSize', { mb: m.download_mb })]);
  lines.push(`## ${t('modules.doc.details')}`, `| ${t('modules.doc.what')} | ${t('modules.doc.value')} |`, '|---|---|', ...rows.map(([a, b]) => `| ${a} | ${b.replace(/\|/g, '/')} |`), '');
  return lines.join('\n');
}
