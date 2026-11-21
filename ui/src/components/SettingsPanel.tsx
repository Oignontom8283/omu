import { useEffect, useRef, useState } from 'react';
import type { api } from '../api.ts';
import { LANGUAGES, languageName, useI18n } from '../i18n/index.tsx';
import { SETTING_DECLS, SETTING_SECTIONS, type SettingDecl } from '../settingsRegistry.ts';
import { applyThemeVars, loadThemeVars, useTheme, type ThemeChoice } from '../theme.tsx';
import { TextInput } from '../kit/controls.tsx';
import type { Settings } from '../types.ts';
import type { Overrides } from '../keybindings.ts';
import { Shortcuts } from './Shortcuts.tsx';

type Props = {
  daemon: ReturnType<typeof api>;
  notify: (text: string) => void;
  /** A setting to show: the page scrolls to it and makes it stand out for a moment. */
  focus?: { id: string; nonce: number };
  /** The shortcuts the person changed, and how to change them. */
  shortcuts: { overrides: Overrides; onChange: (o: Overrides) => void };
};

/** What a setting is worth now, and how it is changed: the page's side of a declaration. */
type Binding = { value: string | boolean; options?: { value: string; label: string }[]; set: (value: string | boolean) => void; disabled?: boolean };

/** The settings of the program itself (not of one block), drawn from the declarations of `settingsRegistry.ts`. */
export function SettingsPanel({ daemon, notify, focus, shortcuts }: Props) {
  const { t, language, setLanguage } = useI18n();
  const { choice, setChoice } = useTheme();
  const [settings, setSettings] = useState<Settings | null>(null);
  const page = useRef<HTMLElement>(null);
  // colours chosen by hand: applied as soon as the text is a valid object of CSS variables
  const [colours, setColours] = useState(loadThemeVars);
  const [colourProblem, setColourProblem] = useState<string | null>(null);
  const changeColours = (text: string) => {
    setColours(text);
    setColourProblem(applyThemeVars(text));
  };
  useEffect(() => {
    applyThemeVars(loadThemeVars());
  }, []);

  useEffect(() => {
    daemon.settings().then(setSettings).catch((e) => notify(e instanceof Error ? e.message : String(e)));
  }, [daemon, notify]);

  const change = async (patch: Partial<Settings>) => {
    if (!settings) return;
    const next = { ...settings, ...patch };
    setSettings(next);
    try {
      await daemon.setSettings(next);
    } catch (e) {
      notify(e instanceof Error ? e.message : String(e));
      setSettings(settings);
    }
  };

  // the page goes to the setting that was asked for
  useEffect(() => {
    if (!focus) return;
    const el = page.current?.querySelector<HTMLElement>(`[data-setting="${CSS.escape(focus.id)}"]`);
    if (!el) return;
    el.scrollIntoView({ block: 'center' });
    el.classList.remove('flash');
    void el.offsetWidth;
    el.classList.add('flash');
    const timer = window.setTimeout(() => el.classList.remove('flash'), 1800);
    return () => window.clearTimeout(timer);
  }, [focus]);

  const bindings: Record<string, Binding> = {
    language: { value: language, options: LANGUAGES.map((code) => ({ value: code, label: languageName(code) })), set: (v) => setLanguage(String(v)) },
    theme: {
      value: choice,
      options: (['system', 'light', 'dark', 'contrast'] as const).map((c) => ({ value: c, label: t(c === 'system' ? 'app.themeSystem' : c === 'light' ? 'app.themeLight' : c === 'dark' ? 'app.themeDark' : 'app.themeContrast') })),
      set: (v) => setChoice(v as ThemeChoice),
    },
    tray: { value: settings?.show_tray_icon ?? false, set: (v) => void change({ show_tray_icon: Boolean(v) }), disabled: !settings },
    logon: { value: settings?.start_at_logon ?? false, set: (v) => void change({ start_at_logon: Boolean(v) }), disabled: !settings },
  };

  const control = (decl: SettingDecl) => {
    const b = bindings[decl.id];
    if (!b) return null;
    if (decl.type === 'toggle') {
      return (
        <label className="check" title={decl.hint ? t(decl.hint) : undefined}>
          <input type="checkbox" checked={Boolean(b.value)} disabled={b.disabled} onChange={(e) => b.set(e.target.checked)} />
          {t(decl.label)}
        </label>
      );
    }
    return (
      <label className="choice">
        <span>{t(decl.label)}</span>
        <select value={String(b.value)} disabled={b.disabled} onChange={(e) => b.set(e.target.value)}>
          {b.options?.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </label>
    );
  };

  return (
    <aside ref={page} className="boards settings" aria-label={t('settings.title')}>
      <header>
        <h2>{t('settings.title')}</h2>
      </header>
      {SETTING_SECTIONS.map((section) => (
        <section key={section.id}>
          <h3>{t(section.label)}</h3>
          {SETTING_DECLS.filter((d) => d.section === section.id).map((decl) => (
            <div key={decl.id} className="setting" data-setting={decl.id}>
              {control(decl)}
              {decl.hint && <small className="muted">{t(decl.hint)}</small>}
            </div>
          ))}
        </section>
      ))}
      <section>
        <h3>{t('settings.section.colours')}</h3>
        <p className="muted">{t('settings.coloursHint')}</p>
        <TextInput value={colours} onChange={changeColours} />
        {colourProblem && <small className="warn">{colourProblem}</small>}
      </section>
      <Shortcuts overrides={shortcuts.overrides} onChange={shortcuts.onChange} />
    </aside>
  );
}
