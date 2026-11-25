// Puts the settings of the program in the registry (once, when the page is first used).

import { LANGUAGES, languageName } from '../i18n/index.tsx';
import { BUILTIN_CATEGORIES, builtinSettings } from './builtin.ts';
import { registerCategory, registerSetting } from './registry.ts';

BUILTIN_CATEGORIES.forEach(registerCategory);
builtinSettings(LANGUAGES.map((code) => ({ value: code, label: languageName(code) }))).forEach(registerSetting);
