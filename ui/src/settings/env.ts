import type { api } from '../api.ts';
import type { SoundImports } from '../imports.tsx';
import type { Overrides } from '../keybindings.ts';
import type { Settings, SoundLibrary } from '../types.ts';

/** What the blocks of the settings page (the sections) need from the rest of the program. */
export type SettingsEnv = {
  daemon: ReturnType<typeof api>;
  notify: (text: string) => void;
  /** What the daemon saves, once it has answered. */
  settings: Settings | null;
  change: (patch: Partial<Settings>) => Promise<void>;
  /** The sounds that were imported, and the imports of the program. */
  library: SoundLibrary;
  setLibrary: (l: SoundLibrary) => void;
  imports: SoundImports;
  /** The shortcuts the person changed, and how to change them. */
  shortcuts: { overrides: Overrides; onChange: (o: Overrides) => void };
  /** Goes to a category of the settings. */
  go: (category: string) => void;
};
