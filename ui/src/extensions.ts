// What an extension can reach from the page: the controls of the program, to use in what it adds or to be taken as a model, and the
// registry where it adds controls of its own. Everything is read through `window.omu` (the same object for every extension).

import * as kit from './kit/controls.tsx';
import { controlNames, controlNamed, registerControl } from './kit/registry.ts';

export type OmuApi = {
  /** The version of this surface: it changes when something an extension uses is taken away. */
  version: 1;
  /** The controls of the program (button, number field, slider, switch, drop-down...), ready to use. */
  kit: typeof kit;
  /** Makes a control available to the blocks under a name of its own (`my-extension.knob`). */
  registerControl: typeof registerControl;
  controlNames: typeof controlNames;
  controlNamed: typeof controlNamed;
};

declare global {
  interface Window {
    omu?: OmuApi;
  }
}

/** Puts the surface on the page. */
export function exposeApi(): void {
  window.omu = { version: 1, kit, registerControl, controlNames, controlNamed };
}
