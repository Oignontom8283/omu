import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ComponentType, type KeyboardEvent } from 'react';
import type { api } from '../api.ts';
import { ContextMenu, SEPARATOR, type MenuItem } from '../components/ContextMenu.tsx';
import { Icon } from '../components/Icon.tsx';
import { useInfoWindows } from '../components/InfoWindows.tsx';
import { Markdown } from '../components/Markdown.tsx';
import { useI18n } from '../i18n/index.tsx';
import type { SoundImports } from '../imports.tsx';
import type { Overrides } from '../keybindings.ts';
import { Button } from '../kit/controls.tsx';
import { ask } from '../kit/dialogs.tsx';
import { FilterBox, Highlight, Pane, SidePane, ToolSeparator, Toolbar } from '../kit/Panes.tsx';
import { cleanFlag, usePersistent } from '../kit/persist.ts';
import { isInteractive } from '../kit/DataTable.tsx';
import { useTheme, type ThemeChoice } from '../theme.tsx';
import type { Settings, SoundLibrary } from '../types.ts';
import { CONTROLS, type ControlProps } from './controls.tsx';
import { categoryDocument, settingDocument } from './doc.ts';
import './register.ts';
import type { SettingsEnv } from './env.ts';
import { onAnyExtensionSetting, readExtensionSetting, writeExtensionSetting } from './extensionValues.ts';
import {
  defaultOf,
  getSnapshot,
  groupsOf,
  isChanged,
  isShown,
  pagesInOrder,
  searchSettings,
  settingsIn,
  subCategories,
  subscribe,
  textOf,
  topCategories,
  type SettingCategory,
  type SettingControl,
  type SettingDecl,
  type SettingValue,
} from './registry.ts';
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

/** Controls that are one short line: the control sits in front of the line that says what the setting does. */
const INLINE_CONTROLS: SettingControl['type'][] = ['toggle'];
/** What a search for changed settings is typed as (as in the settings of a code editor). */
const CHANGED_WORD = '@modified';

/** What is chosen in the list: a setting, or the head of a page. */
type Pick = { kind: 'setting' | 'page'; id: string } | null;

/**
 * The settings of the program and of the extensions, laid out like the settings editor of an IDE. On the left the tree of the categories,
 * which follows the list as it scrolls; in the middle every page one after the other, each setting with its name, a line about what it does,
 * a mark when it was changed, a button that puts it back and an "i" for its documentation; on the right the documentation of what is chosen.
 * The search filters the list where it is and marks the words it found; `@modified` (or the button) keeps only what was changed.
 */
export function SettingsPage({ daemon, notify, library, setLibrary, imports, focus, shortcuts }: Props) {
  const { t, language, setLanguage } = useI18n();
  const { choice, setChoice } = useTheme();
  const state = useSyncExternalStore(subscribe, getSnapshot);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [query, setQuery] = useState('');
  const [current, setCurrent] = useState<string | undefined>(undefined);
  const [pick, setPick] = useState<Pick>(null);
  const [docs, setDocs] = usePersistent('omu-settings-docs', true, cleanFlag);
  const [closed, setClosed] = usePersistent<string[]>('omu-settings-closed', [], (v) => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : undefined));
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const [focusFilter, setFocusFilter] = useState(0);
  const [focusTree, setFocusTree] = useState(0);
  const list = useRef<HTMLDivElement>(null);
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
  const changed = (decl: SettingDecl) => decl.bind.source !== 'daemon' || settings ? isChanged(decl, valueOf(decl)) : false;
  const setValue = (decl: SettingDecl, value: SettingValue) => {
    const bind = decl.bind;
    if (bind.source === 'daemon') void change({ [bind.key]: value });
    else if (bind.source === 'ui') {
      if (bind.key === 'language') setLanguage(String(value));
      else setChoice(String(value) as ThemeChoice);
    } else if (bind.source === 'extension') writeExtensionSetting(bind.extension, bind.key, value);
  };
  const reset = (decl: SettingDecl) => {
    const usual = defaultOf(decl);
    if (usual !== undefined) setValue(decl, usual);
  };

  // ---- what the list shows: every page in the order of the tree, each with the settings the search and the filter keep
  const onlyChanged = query.split(/\s+/).includes(CHANGED_WORD);
  const words = query
    .split(/\s+/)
    .filter((w) => w !== CHANGED_WORD)
    .join(' ')
    .trim();
  const found = useMemo(() => (words === '' ? null : new Set(searchSettings(state, words, t, shown).map((s) => s.id))), [state, words, t, settings, language, choice]); // eslint-disable-line react-hooks/exhaustive-deps
  const keep = (decl: SettingDecl) => shown(decl) && (!found || found.has(decl.id)) && (!onlyChanged || changed(decl));
  const filtering = found !== null || onlyChanged;
  const pages = pagesInOrder(state, t).map((page) => ({ page, items: settingsIn(state, page.id).filter(keep) }));
  const visiblePages = filtering ? pages.filter((p) => p.items.length > 0) : pages;
  const rows = visiblePages.flatMap((p) => p.items);
  const allChanged = state.settings.filter((d) => shown(d) && changed(d));

  // what each line of the tree shows: the settings found in it, or the ones changed in it
  const badges: Record<string, { count: number; changed: boolean } | undefined> = {};
  for (const { page, items } of pages) {
    const count = filtering ? items.length : settingsIn(state, page.id).filter((d) => shown(d) && changed(d)).length;
    badges[page.id] = { count, changed: !filtering };
  }
  for (const top of topCategories(state, t)) {
    const subs = subCategories(state, top.id, t);
    if (subs.length === 0) continue;
    const count = subs.reduce((n, s) => n + (badges[s.id]?.count ?? 0), 0);
    badges[top.id] = { count, changed: !filtering };
  }

  const name = (c: SettingCategory) => textOf(t, c.plain, c.title);
  const titleOf = (decl: SettingDecl) => textOf(t, decl.plain, decl.title);

  // ---- going somewhere: the tree scrolls the list to a page, and the list scrolling tells the tree where it is
  const scrollTo = (page: string) => {
    const box = list.current;
    const el = box?.querySelector<HTMLElement>(`[data-page="${CSS.escape(page)}"]`);
    if (box && el) box.scrollTop += el.getBoundingClientRect().top - box.getBoundingClientRect().top;
  };
  const go = (id: string) => {
    const category = state.categories.find((c) => c.id === id);
    if (!category) return;
    const first = subCategories(state, id, t)[0] ?? category;
    if (first.parent) setClosed((c) => c.filter((x) => x !== first.parent));
    setCurrent(first.id);
    setPick({ kind: 'page', id: first.id });
    // the list is drawn again without the search first when the page is not in it
    if (!visiblePages.some((p) => p.page.id === first.id)) setQuery('');
    window.setTimeout(() => scrollTo(first.id), 0);
  };
  const onScroll = useCallback(() => {
    const box = list.current;
    if (!box) return;
    const top = box.getBoundingClientRect().top + 8;
    let at: string | undefined;
    for (const el of box.querySelectorAll<HTMLElement>('[data-page]')) {
      if (el.getBoundingClientRect().top <= top) at = el.dataset.page;
      else break;
    }
    at ??= box.querySelector<HTMLElement>('[data-page]')?.dataset.page;
    setCurrent((c) => (c === at ? c : at));
  }, []);
  useEffect(onScroll, [onScroll, query, state]);

  // the page goes to what was asked for: a category, or a setting (which then stands out for a moment)
  useEffect(() => {
    if (!focus) return;
    if (state.categories.some((c) => c.id === focus.id)) return go(focus.id);
    const decl = state.settings.find((s) => s.id === focus.id);
    if (!decl) return;
    setQuery('');
    setPick({ kind: 'setting', id: decl.id });
    const parent = state.categories.find((c) => c.id === decl.category)?.parent;
    if (parent) setClosed((c) => c.filter((x) => x !== parent));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [focus]);
  useEffect(() => {
    if (!focus || flashed.current === focus.nonce) return;
    const el = list.current?.querySelector<HTMLElement>(`[data-setting="${CSS.escape(focus.id)}"]`);
    if (!el) return;
    flashed.current = focus.nonce;
    el.scrollIntoView({ block: 'center' });
    el.classList.remove('flash');
    void el.offsetWidth;
    el.classList.add('flash');
    const timer = window.setTimeout(() => el.classList.remove('flash'), 1800);
    return () => window.clearTimeout(timer);
  });

  // ---- the documentation: in the pane on the right for what is chosen, and in floating windows from the "i" buttons
  const docOf = (p: Pick): string | null => {
    if (!p) return null;
    if (p.kind === 'setting') {
      const decl = state.settings.find((s) => s.id === p.id);
      return decl ? settingDocument(state, decl, t).source : null;
    }
    const category = state.categories.find((c) => c.id === p.id);
    return category ? categoryDocument(state, category, t, valueById).source : null;
  };
  const info = useInfoWindows((key) => {
    const at = key.indexOf(':');
    const source = docOf({ kind: key.startsWith('setting:') ? 'setting' : 'page', id: key.slice(at + 1) });
    return source ? { source } : null;
  }, { setting: (id) => `setting:${id}` });
  const about = (key: string, label: string) => (
    <Button variant="flat" size="small" icon="help" title={t('settings.about', { name: label })} onClick={(e) => info.open(key, { x: e.clientX + 12, y: e.clientY - 8 })} />
  );
  const showSetting = (id: string) => {
    const decl = state.settings.find((s) => s.id === id);
    if (!decl) return;
    if (!rows.includes(decl)) setQuery('');
    setPick({ kind: 'setting', id });
    window.setTimeout(() => list.current?.querySelector<HTMLElement>(`[data-setting="${CSS.escape(id)}"]`)?.scrollIntoView({ block: 'center' }), 0);
  };

  // ---- the menu of a setting, and putting everything back
  const copy = (text: string) => void navigator.clipboard.writeText(text).catch((e) => notify(e instanceof Error ? e.message : String(e)));
  const settingMenu = (e: React.MouseEvent | { clientX: number; clientY: number }, decl: SettingDecl) => {
    setPick({ kind: 'setting', id: decl.id });
    setMenu({
      x: e.clientX,
      y: e.clientY,
      items: [
        { label: t('settings.reset'), disabled: !changed(decl), run: () => reset(decl) },
        SEPARATOR,
        { label: t('settings.copyValue'), disabled: decl.control.type === 'section' || decl.control.type === 'custom', run: () => copy(String(valueOf(decl))) },
        SEPARATOR,
        { label: t('settings.openDoc'), run: () => info.open(`setting:${decl.id}`, { x: e.clientX + 12, y: e.clientY }) },
      ],
    });
  };
  const resetAll = async (e: React.MouseEvent) => {
    if (allChanged.length === 0) return;
    if (!(await ask({ title: t('settings.resetAll.title', { count: allChanged.length }), message: allChanged.map(titleOf).join(', ') }, e))) return;
    allChanged.forEach(reset);
  };

  // ---- the keyboard: the arrows go from setting to setting, Enter goes into its control, Escape comes back out of it
  const rowKeys = (e: KeyboardEvent<HTMLDivElement>, decl: SettingDecl) => {
    const row = e.currentTarget;
    if (isInteractive(e.target) || e.target !== row) {
      if (e.key === 'Escape' && e.target !== row) {
        e.preventDefault();
        row.focus();
      }
      return;
    }
    const all = [...(list.current?.querySelectorAll<HTMLElement>('.sp-row') ?? [])];
    const at = all.indexOf(row);
    const goTo = (i: number) => {
      const el = all[Math.max(0, Math.min(all.length - 1, i))];
      if (!el) return;
      el.focus();
      setPick({ kind: 'setting', id: el.dataset.setting! });
    };
    switch (e.key) {
      case 'ArrowDown':
        goTo(at + 1);
        break;
      case 'ArrowUp':
        goTo(at - 1);
        break;
      case 'Home':
        goTo(0);
        break;
      case 'End':
        goTo(all.length - 1);
        break;
      case 'Enter':
        row.querySelector<HTMLElement>('.sp-control button, .sp-control input, .sp-control [tabindex="0"], .sp-control select')?.focus();
        break;
      case 'Delete':
      case 'Backspace':
        if (changed(decl)) reset(decl);
        break;
      case 'ContextMenu':
      case 'F10': {
        if (e.key === 'F10' && !e.shiftKey) return;
        const r = row.getBoundingClientRect();
        settingMenu({ clientX: r.left + 40, clientY: r.top + 24 }, decl);
        break;
      }
      default:
        return;
    }
    e.preventDefault();
  };
  // Ctrl+F looks in the settings, Ctrl+Shift+E goes to the tree
  const pageKeys = (e: KeyboardEvent) => {
    const k = e.key.toLowerCase();
    if ((e.ctrlKey || e.metaKey) && !e.altKey && k === 'f') {
      e.preventDefault();
      setFocusFilter((n) => n + 1);
    } else if ((e.ctrlKey || e.metaKey) && e.shiftKey && k === 'e') {
      e.preventDefault();
      setFocusTree((n) => n + 1);
    }
  };

  const env: SettingsEnv = { daemon, notify, settings, change, library, setLibrary, imports, shortcuts, go };

  /** One setting: a mark when it was changed, its name with its buttons, what it does (the switch in front of it), then its control. */
  const item = (decl: SettingDecl, alone: boolean) => {
    const Control = CONTROLS[decl.control.type] as ComponentType<ControlProps<SettingControl['type']>>;
    const title = titleOf(decl);
    const description = decl.description ? textOf(t, decl.plain, decl.description) : undefined;
    const control = <Control control={decl.control} decl={decl} label={title} value={valueOf(decl)} set={(v) => setValue(decl, v)} disabled={decl.bind.source === 'daemon' && !settings} t={t} env={env} />;
    const inline = INLINE_CONTROLS.includes(decl.control.type);
    const isChangedNow = changed(decl);
    const on = pick?.kind === 'setting' && pick.id === decl.id;
    const wide = decl.control.type === 'section' || decl.control.type === 'custom';
    // a block that is the whole page is shown without its name (the head of the page has it), unless a search shows it alone
    const bare = decl.bare && alone && !filtering;
    return (
      <div
        key={decl.id}
        className={`sp-row${isChangedNow ? ' changed' : ''}${on ? ' selected' : ''}${wide ? ' wide' : ''}${bare ? ' bare' : ''}`}
        data-setting={decl.id}
        tabIndex={on || (!pick && rows[0] === decl) ? 0 : -1}
        aria-label={title}
        onMouseDown={(e) => {
          if (pick?.id !== decl.id || pick.kind !== 'setting') setPick({ kind: 'setting', id: decl.id });
          if (!isInteractive(e.target) && e.button === 0) (e.currentTarget as HTMLElement).focus({ preventScroll: true });
        }}
        onContextMenu={(e) => {
          if (isInteractive(e.target) && (e.target as HTMLElement).closest('input, textarea')) return;
          e.preventDefault();
          settingMenu(e, decl);
        }}
        onKeyDown={(e) => rowKeys(e, decl)}
      >
        <span className="sp-mark" title={isChangedNow ? t('settings.changedMark') : undefined} />
        {!bare && (
          <div className="sp-head">
            <span className="sp-title">
              <Highlight text={title} query={words} />
            </span>
            <span className="sp-tools">
              {isChangedNow && <Button variant="flat" size="small" icon="undo" title={t('settings.reset')} onClick={() => reset(decl)} />}
              {about(`setting:${decl.id}`, title)}
              <Button variant="flat" size="small" icon="more" title={t('settings.more')} onClick={(e) => settingMenu(e, decl)} />
            </span>
          </div>
        )}
        {!bare && inline ? (
          <label className="sp-inline">
            <span className="sp-control">{control}</span>
            {description && (
              <span className="sp-desc">
                <Highlight text={description} query={words} />
              </span>
            )}
          </label>
        ) : (
          <>
            {!bare && description && (
              <p className="sp-desc">
                <Highlight text={description} query={words} />
              </p>
            )}
            <div className="sp-control">{control}</div>
          </>
        )}
      </div>
    );
  };

  // what is chosen, while it is in the list; otherwise the page the list is on
  const pickShown = pick && (pick.kind === 'setting' ? rows.some((d) => d.id === pick.id) : visiblePages.some((p) => p.page.id === pick.id));
  const docSource = docOf(pickShown ? pick : current ? { kind: 'page', id: current } : null);
  const counted = filtering ? t('settings.results', { count: rows.length }) : undefined;

  return (
    <div className="ide-page settings-page" aria-label={t('settings.title')} onKeyDown={pageKeys}>
      <Toolbar label={t('settings.toolbar')}>
        <FilterBox value={query} onChange={setQuery} placeholder={t('settings.search')} count={counted} focusKey={focusFilter} />
        <Button
          size="small"
          icon="filter"
          pressed={onlyChanged}
          title={t('settings.onlyChangedHint', { word: CHANGED_WORD })}
          onClick={() => setQuery((q) => (onlyChanged ? q.split(/\s+/).filter((w) => w !== CHANGED_WORD).join(' ') : `${CHANGED_WORD} ${q}`.trim()))}
        >
          {t('settings.onlyChanged')}
        </Button>
        <span className="tool-text">{t('settings.changedCount', { count: allChanged.length })}</span>
        <span className="spacer" />
        <Button size="small" icon="undo" disabled={allChanged.length === 0} title={t('dialog.shiftHint')} onClick={(e) => void resetAll(e)}>
          {t('settings.resetAll')}
        </Button>
        <ToolSeparator />
        <Button size="small" icon="unfold" title={t('tree.expandAll')} onClick={() => setClosed([])} />
        <Button size="small" icon="collapse" title={t('tree.collapseAll')} onClick={() => setClosed(topCategories(state, t).map((c) => c.id))} />
        <Button size="small" icon="sideRight" pressed={docs} title={t('settings.docsPane')} onClick={() => setDocs(!docs)} />
      </Toolbar>
      <div className="ide-columns">
        <SidePane id="settings-tree" side="left" usual={240} min={160} max={480}>
          <Pane title={t('settings.categories')}>
            <SettingsTree
              state={state}
              t={t}
              label={t('settings.categories')}
              selected={current}
              expanded={topCategories(state, t)
                .map((c) => c.id)
                .filter((id) => !closed.includes(id))}
              onSelect={go}
              onToggle={(id) => setClosed((c) => (c.includes(id) ? c.filter((x) => x !== id) : [...c, id]))}
              badges={badges}
              query={words}
              focusKey={focusTree}
            />
          </Pane>
        </SidePane>
        <div className="ide-center">
          <div ref={list} className="sp-list" onScroll={onScroll}>
            {visiblePages.length === 0 && (
              <p className="pane-empty">{onlyChanged && !found ? t('settings.noneChanged') : t('settings.noResults', { query: words || CHANGED_WORD })}</p>
            )}
            {visiblePages.map(({ page, items }, index) => {
              const parent = page.parent ? state.categories.find((c) => c.id === page.parent) : undefined;
              const groups = groupsOf(items);
              // the first page of a category opens with the name of the category, as a chapter
              const before = visiblePages[index - 1]?.page;
              const opensCategory = parent && (!before || before.parent !== page.parent);
              return (
                <section key={page.id} className="sp-page" data-page={page.id} aria-label={name(page)}>
                  {opensCategory && (
                    <h2 className="sp-category">
                      {parent.icon && <Icon name={parent.icon} size={18} />}
                      {name(parent)}
                    </h2>
                  )}
                  <header className={`sp-page-head${pick?.kind === 'page' && pick.id === page.id ? ' selected' : ''}`} onMouseDown={() => setPick({ kind: 'page', id: page.id })}>
                    <div className="sp-page-title">
                      <h3>{name(page)}</h3>
                      {about(`category:${page.id}`, name(page))}
                    </div>
                    {page.description && <p className="sp-page-desc">{textOf(t, page.plain, page.description)}</p>}
                  </header>
                  {groups.map((group, i) => (
                    <div key={group.title?.text ?? `group-${i}`} className="sp-group">
                      {group.title && groups.length > 1 && <h4 className="sp-group-title">{textOf(t, group.title.plain, group.title.text)}</h4>}
                      {group.items.map((decl) => item(decl, items.length === 1))}
                    </div>
                  ))}
                  {page.parent === 'extensions' && page.id !== 'installed' && items.length === 0 && <p className="pane-empty">{t('settings.extensions.noSettings')}</p>}
                </section>
              );
            })}
          </div>
        </div>
        {docs && (
          <SidePane id="settings-docs" side="right" usual={340} min={220} max={720}>
            <Pane
              title={t('settings.docsPane')}
              actions={<Button variant="flat" size="small" icon="close" title={t('app.close')} onClick={() => setDocs(false)} />}
            >
              <div className="sp-doc">{docSource ? <Markdown source={docSource} links={{ setting: showSetting }} /> : <p className="pane-empty">{t('settings.docsNone')}</p>}</div>
            </Pane>
          </SidePane>
        )}
      </div>
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
      {info.windows}
    </div>
  );
}
