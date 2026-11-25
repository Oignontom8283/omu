// What an extension uses to appear in the settings: it declares itself (its category, with its name and its description), then its
// settings; the page draws them like the ones of the program. The values are kept for it, and it reads them with `getSetting`.

import { onExtensionSetting, readExtensionSetting } from './extensionValues.ts';
import { registerCategory, registerSetting, type SettingControl, type SettingValue } from './registry.ts';

export type ExtensionInfo = {
  /** A name of its own (`my-extension`): it starts the id of every setting of the extension. */
  id: string;
  name: string;
  /** What it is for, in a line (under the title of its page). */
  description?: string;
  /** Its documentation in the little markdown of the program (headings, lists, tables, `code`, **bold**): the information window of its page. */
  doc?: string;
};

export type ExtensionSetting = {
  /** The id of the extension that owns it. */
  extension: string;
  /** Its own id inside the extension (`volume`). */
  id: string;
  title: string;
  /** What it does, in a line (under the title). */
  description?: string;
  /** Its documentation in the little markdown of the program: the information window of the setting. */
  doc?: string;
  /** The ids (`volume`) of the other settings of the same extension that go with it: the information window links to them. */
  related?: string[];
  keywords?: string;
  control: Exclude<SettingControl, { type: 'section' }>;
  /** What it is until someone changes it (its type is the type of the value: a text, a number, a yes/no). */
  default: SettingValue;
};

/** The id of the category of an extension. */
export const extensionCategory = (extension: string) => `extension:${extension}`;

/** Declares an extension: it gets a category of its own under "Extensions". Returns what takes it away, with its settings. */
export function registerExtension(info: ExtensionInfo): () => void {
  return registerCategory({ id: extensionCategory(info.id), title: info.name, description: info.description, doc: info.doc, plain: true, parent: 'extensions', order: 100 });
}

/** Adds a setting to the category of an extension. Returns what takes it away. */
export function registerExtensionSetting(setting: ExtensionSetting): () => void {
  return registerSetting({
    id: `${extensionCategory(setting.extension)}:${setting.id}`,
    category: extensionCategory(setting.extension),
    title: setting.title,
    description: setting.description,
    doc: setting.doc,
    related: setting.related?.map((id) => `${extensionCategory(setting.extension)}:${id}`),
    keywords: setting.keywords,
    plain: true,
    control: setting.control,
    bind: { source: 'extension', extension: setting.extension, key: setting.id, default: setting.default },
  });
}

/** The value of a setting of an extension. */
export const getSetting = (extension: string, id: string, fallback: SettingValue): SettingValue => readExtensionSetting(extension, id, fallback);

/** Calls `listener` each time the setting changes. Returns what stops it. */
export const onSettingChange = onExtensionSetting;
