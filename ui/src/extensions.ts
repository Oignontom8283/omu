// What an extension can reach from the page: the controls of the program, to use in what it adds or to be taken as a model, and the
// registry where it adds controls of its own. Everything is read through `window.omu` (the same object for every extension).

import * as kit from './kit/controls.tsx';
import { controlNames, controlNamed, registerControl } from './kit/registry.ts';
import { registerCommand, registerPanel } from './extensionRegistry.ts';
import { getSetting, onSettingChange, registerExtension, registerExtensionSetting } from './settings/extensionApi.ts';

export type OmuApi = {
  /** The version of this surface: it changes when something an extension uses is taken away. */
  version: 3;
  /** The controls of the program (button, number field, slider, switch, drop-down...), ready to use. */
  kit: typeof kit;
  /** Makes a control available to the blocks under a name of its own (`my-extension.knob`). */
  registerControl: typeof registerControl;
  controlNames: typeof controlNames;
  controlNamed: typeof controlNamed;
  /** Adds a command to the command palette. Returns what takes it away again. */
  registerCommand: typeof registerCommand;
  /** Adds a panel (a floating window the extension fills), opened from the command palette. Returns what takes it away. */
  registerPanel: typeof registerPanel;
  /** Declares the extension in the settings: it gets a category of its own under "Extensions". Returns what takes it away, with its settings. */
  registerExtension: typeof registerExtension;
  /** Adds a setting to the category of an extension (a switch, a choice, a text, a number, a slider, or a control of its own). Returns what takes it away. */
  registerSetting: typeof registerExtensionSetting;
  /** The value of a setting of an extension: what the person set, or the default. */
  getSetting: typeof getSetting;
  /** Calls a function each time a setting of an extension changes. Returns what stops it. */
  onSettingChange: typeof onSettingChange;
};

declare global {
  interface Window {
    omu?: OmuApi;
  }
}

/** Puts the surface on the page. */
export function exposeApi(): void {
  window.omu = { version: 3, kit, registerControl, controlNames, controlNamed, registerCommand, registerPanel, registerExtension, registerSetting: registerExtensionSetting, getSetting, onSettingChange };
}
