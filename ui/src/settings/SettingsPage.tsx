import { useEffect, useMemo, useRef, useState, useSyncExternalStore, type ComponentType } from 'react';
import type { api } from '../api.ts';
import { Icon } from '../components/Icon.tsx';
import { useInfoWindows } from '../components/InfoWindows.tsx';
import { useI18n } from '../i18n/index.tsx';
import type { SoundImports } from '../imports.tsx';
import type { Overrides } from '../keybindings.ts';
import { Button, TextInput } from '../kit/controls.tsx';
import { useTheme, type ThemeChoice } from '../theme.tsx';
import type { Settings, SoundLibrary } from '../types.ts';
import { CONTROLS, type ControlProps } from './controls.tsx';
import { categoryDocument, settingDocument } from './doc.ts';
import './register.ts';
import type { SettingsEnv } from './env.ts';
import { onAnyExtensionSetting, readExtensionSetting, writeExtensionSetting } from './extensionValues.ts';
import { getSnapshot, groupsOf, isShown, pageOf, searchSettings, settingsIn, subscribe, textOf, topCategories, type SettingCategory, type SettingControl, type SettingDecl, type SettingValue } from './registry.ts';
import { SettingsTree } from './SettingsTree.tsx';

type Props = {
  daemon: ReturnType<typeof api>;
  notify: (text: string) => void;
  /** The sounds that were imported, and the imports of the program. */
  library: SoundLibrary;
  setLibrary: (l: SoundLibrary) => void;
  imports: SoundImports;
  /** A setting or a category to show: the page goes to it and makes a setting stand out for a moment. */
  focus?: { id: string; nonce: number };
  /** The shortcuts the person changed, and how to change them. */
  shortcuts: { overrides: Overrides; onChange: (o: Overrides) => void };
};

/** Controls that are one short line: the title and the control share a row (a switch); the others sit under the title and what it does. */
const INLINE_CONTROLS: SettingControl['type'][] = ['toggle'];

/**
 * The settings of the program and of the extensions. On the left a search and the tree of the categories; on the right the page that is
 * chosen: its title, then its settings, each with its name, a line about what it does, an "i" that opens its documentation in a floating window,
 * and its control. Everything on it comes from what is registered in `registry.ts`; the controls are components found by the type of each setting
 * (`controls.tsx`), and the blocks that need more than a control are sections (`sections.tsx`).
 */
export function SettingsPage({ daemon, notify, library, setLibrary, imports, focus, shortcuts }: Props) {
  const { t, language, setLanguage } = useI18n();
  const { choice, setChoice } = useTheme();
  const state = useSyncExternalStore(subscribe, getSnapshot);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [selected, setSelected] = useState('appearance');
  const [closed, setClosed] = useState<string[]>([]);
  const [query, setQuery] = useState('');
  const main = useRef<HTMLDivElement>(null);
  const flashed = useRef(0);
  // the values of the extensions' settings are kept elsewhere: the page is drawn again when one changes
  const [, redraw] = useState(0);
  useEffect(() => onAnyExtensionSetting(() => redraw((n) => n + 1)), []);

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

  /** Goes to a page (a heading goes to its first page, and is opened). */
  const go = (id: string) => {
    const page = pageOf(state, id, t);
    if (!page) return;
    setQuery('');
    setSelected(page.id);
    if (page.parent) setClosed((c) => c.filter((x) => x !== page.parent));
    main.current?.scrollTo({ top: 0 });
  };

  // the page goes to what was asked for: a category, or the category of a setting (which then stands out for a moment)
  useEffect(() => {
    if (!focus) return;
    const category = state.categories.find((c) => c.id === focus.id);
    const decl = state.settings.find((s) => s.id === focus.id);
    if (category) go(category.id);
    else if (decl) go(decl.category);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focus]);
  useEffect(() => {
    if (!focus || flashed.current === focus.nonce) return;
    const el = main.current?.querySelector<HTMLElement>(`[data-setting="${CSS.escape(focus.id)}"]`);
    if (!el) return;
    flashed.current = focus.nonce;
    el.scrollIntoView({ block: 'center' });
    el.classList.remove('flash');
    void el.offsetWidth;
    el.classList.add('flash');
    const timer = window.setTimeout(() => el.classList.remove('flash'), 1800);
    return () => window.clearTimeout(timer);
  }, [focus, selected]);

  // what a setting is worth now, and how it is changed, whatever the place it is kept in
  const valueOf = (decl: SettingDecl): SettingValue => {
    const bind = decl.bind;
    switch (bind.source) {
      case 'daemon': {
        const v = (settings as Record<string, unknown> | null)?.[bind.key];
        return typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean' ? v : decl.control.type === 'toggle' ? false : '';
      }
      case 'ui':
        return bind.key === 'language' ? language : choice;
      case 'extension':
        return readExtensionSetting(bind.extension, bind.key, bind.default);
      case 'none':
        return '';
    }
  };
  const valueById = (id: string): SettingValue | undefined => {
    const decl = state.settings.find((s) => s.id === id);
    return decl ? valueOf(decl) : undefined;
  };
  const shown = (decl: SettingDecl) => isShown(decl, valueById);
  const setValue = (decl: SettingDecl, value: SettingValue) => {
    const bind = decl.bind;
    if (bind.source === 'daemon') void change({ [bind.key]: value });
    else if (bind.source === 'ui') {
      if (bind.key === 'language') setLanguage(String(value));
      else setChoice(String(value) as ThemeChoice);
    } else if (bind.source === 'extension') writeExtensionSetting(bind.extension, bind.key, value);
  };

  // the information windows: the documentation of a setting or of a page, each in a window of its own
  const info = useInfoWindows(
    (key) => {
      const at = key.indexOf(':');
      const id = key.slice(at + 1);
      if (key.startsWith('setting:')) {
        const decl = state.settings.find((s) => s.id === id);
        return decl ? { source: settingDocument(state, decl, t).source } : null;
      }
      const category = state.categories.find((c) => c.id === id);
      return category ? { source: categoryDocument(state, category, t, valueById).source } : null;
    },
    { setting: (id) => `setting:${id}` },
  );
  const about = (key: string, name: string) => (
    <Button variant="flat" size="small" icon="help" title={t('settings.about', { name })} onClick={(e) => info.open(key, { x: e.clientX + 12, y: e.clientY - 8 })} />
  );

  const env: SettingsEnv = { daemon, notify, settings, change, library, setLibrary, imports, shortcuts, go };

  // the page that is shown (the first one when the chosen one is gone: an extension was taken away)
  const first = topCategories(state, t)[0];
  const current = state.categories.find((c) => c.id === selected && !state.categories.some((x) => x.parent === c.id)) ?? (first ? pageOf(state, first.id, t) : undefined);
  const parent = current?.parent ? state.categories.find((c) => c.id === current.parent) : undefined;
  const found = useMemo(() => (query.trim() === '' ? null : searchSettings(state, query, t, shown)), [state, query, t, settings, language, choice]);
  const name = (c: SettingCategory) => textOf(t, c.plain, c.title);

  /** One setting: its name with its "i", what it does, then its control (beside the name for a switch, under it for the others). */
  const item = (decl: SettingDecl, alone = false) => {
    const Control = CONTROLS[decl.control.type] as ComponentType<ControlProps<SettingControl['type']>>;
    const title = textOf(t, decl.plain, decl.title);
    const description = decl.description ? textOf(t, decl.plain, decl.description) : undefined;
    const control = <Control control={decl.control} decl={decl} label={title} value={valueOf(decl)} set={(v) => setValue(decl, v)} disabled={decl.bind.source === 'daemon' && !settings} t={t} env={env} />;
    const inline = INLINE_CONTROLS.includes(decl.control.type);
    // a block that is the whole page is shown without its name (the page has it), except in the results of a search where nothing else says what it is
    if (decl.bare && alone) return <div key={decl.id} className="setting-item bare" data-setting={decl.id}>{control}</div>;
    return (
      <div key={decl.id} className={`setting-item${inline ? ' inline' : ''}`} data-setting={decl.id}>
        <div className="setting-text">
          <div className="setting-head">
            <h3 className="setting-title">{title}</h3>
            {about(`setting:${decl.id}`, title)}
          </div>
          {description && <p className="setting-desc">{description}</p>}
        </div>
        <div className="setting-control">{control}</div>
      </div>
    );
  };

  const groups = current ? groupsOf(settingsIn(state, current.id).filter(shown)) : [];
  return (
    <div className="page settings-page" aria-label={t('settings.title')}>
      <aside className="settings-side">
        <div className="settings-search">
          <Icon name="search" size={14} />
          <TextInput code={false} placeholder={t('settings.search')} title={t('settings.search')} value={query} onChange={setQuery} />
          {query && <Button variant="flat" size="small" icon="close" title={t('settings.clearSearch')} onClick={() => setQuery('')} />}
        </div>
        <SettingsTree
          state={state}
          t={t}
          label={t('settings.categories')}
          selected={found ? undefined : current?.id}
          closed={closed}
          onSelect={go}
          onToggle={(id) => setClosed((c) => (c.includes(id) ? c.filter((x) => x !== id) : [...c, id]))}
        />
      </aside>

      <div ref={main} className="settings-main">
        <div className="page-inner">
          {found ? (
            found.length === 0 ? (
              <p className="muted">{t('settings.noResults', { query: query.trim() })}</p>
            ) : (
              <>
                <h1>{t('settings.results', { count: found.length })}</h1>
                {found.map((d) => {
                  const c = state.categories.find((x) => x.id === d.category);
                  const p = c?.parent ? state.categories.find((x) => x.id === c.parent) : undefined;
                  return (
                    <section key={d.id} className="setting-result">
                      {c && (
                        <button type="button" className="setting-crumb" onClick={() => go(c.id)}>
                          {p ? `${name(p)} › ` : ''}
                          {name(c)}
                        </button>
                      )}
                      {item(d)}
                    </section>
                  );
                })}
              </>
            )
          ) : (
            current && (
              <>
                <header className="page-head">
                  {parent && <div className="page-crumbs">{name(parent)}</div>}
                  <div className="page-title">
                    <h1>{name(current)}</h1>
                    {about(`category:${current.id}`, name(current))}
                  </div>
                  {current.description && <p className="page-lead">{textOf(t, current.plain, current.description)}</p>}
                </header>
                {groups.map((group, i) => (
                  <section key={group.title?.text ?? `group-${i}`} className="setting-group">
                    {group.title && groups.length > 1 && <h2>{textOf(t, group.title.plain, group.title.text)}</h2>}
                    {group.items.map((decl) => item(decl, true))}
                  </section>
                ))}
                {current.parent === 'extensions' && current.id !== 'installed' && groups.length === 0 && <p className="muted">{t('settings.extensions.noSettings')}</p>}
              </>
            )
          )}
        </div>
      </div>
      {info.windows}
    </div>
  );
}
