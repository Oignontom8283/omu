/**
 * The settings of the program, declared once. The settings page is drawn from these declarations and the command palette
 * searches them: a setting is added here (and given its value in the page), never written into the page by hand.
 */
export type SettingDecl = {
  id: string;
  section: 'appearance' | 'system' | 'security';
  /** A yes/no, or one choice among several. */
  type: 'toggle' | 'choice';
  /** Translation keys of its name and of its explanation. */
  label: string;
  hint?: string;
};

export const SETTING_SECTIONS: { id: SettingDecl['section']; label: string }[] = [
  { id: 'appearance', label: 'settings.section.appearance' },
  { id: 'system', label: 'settings.section.system' },
  { id: 'security', label: 'settings.section.security' },
];

export const SETTING_DECLS: SettingDecl[] = [
  { id: 'language', section: 'appearance', type: 'choice', label: 'app.language' },
  { id: 'theme', section: 'appearance', type: 'choice', label: 'app.theme' },
  { id: 'tray', section: 'system', type: 'toggle', label: 'settings.tray', hint: 'settings.trayHint' },
  { id: 'logon', section: 'system', type: 'toggle', label: 'settings.logon', hint: 'settings.logonHint' },
  { id: 'trust', section: 'security', type: 'choice', label: 'settings.trust', hint: 'settings.trustHint' },
];
