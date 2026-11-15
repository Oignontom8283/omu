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
