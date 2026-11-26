import { useMemo, useState } from 'react';
import { useT } from '../i18n/index.tsx';
import { COMMANDS, bindings, commandsFor, saveOverrides, type CommandId, type Overrides } from '../keybindings.ts';
import { comboFromEvent } from '../graph.ts';
import { DataTable, type Column } from '../kit/DataTable.tsx';
import { FilterBox, Highlight, Toolbar, foldText } from '../kit/Panes.tsx';
import { Button } from '../kit/controls.tsx';
import { NOTHING, type ListSelection } from '../kit/selectionModel.ts';
import { ContextMenu, SEPARATOR, type MenuItem } from './ContextMenu.tsx';
import { Icon } from './Icon.tsx';

type Row = { id: CommandId; label: string; keys: string; usual: string; changed: boolean; clash: boolean };

/** The place where the next keys pressed become the shortcut: Escape gives up, Backspace takes the shortcut away. */
function KeyCapture({ onDone }: { onDone: (keys: string | null) => void }) {
  const t = useT();
  return (
    <span
      className="key-capture listening"
      tabIndex={0}
      role="textbox"
      aria-label={t('keys.press')}
      ref={(el) => el?.focus()}
      onBlur={() => onDone(null)}
      onMouseDown={(e) => e.stopPropagation()}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === 'Tab') return onDone(null);
        e.preventDefault();
        if (e.key === 'Escape') return onDone(null);
        if (e.key === 'Backspace' || e.key === 'Delete') return onDone('');
        const combo = comboFromEvent(e.nativeEvent);
        if (combo) onDone(combo);
      }}
    >
      {t('keys.press')}
    </span>
  );
}

/**
 * Every command and its shortcut, as the shortcuts editor of a code editor: a table that is filtered by the name of the command or by its
 * keys, sorted by any column; a double click or Enter records new keys, Delete takes them away, and the menu puts the usual ones back.
 * Two commands on the same keys are flagged. What is changed is kept in this browser.
 */
export function Shortcuts({ overrides, onChange }: { overrides: Overrides; onChange: (o: Overrides) => void }) {
  const t = useT();
  const [query, setQuery] = useState('');
  const [selection, setSelection] = useState<ListSelection>(NOTHING);
  const [recording, setRecording] = useState<CommandId | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null);
  const map = bindings(overrides);
  const set = (id: CommandId, keys: string) => {
    const next = { ...overrides, [id]: keys };
    // back to the shipped one: nothing to remember
    if (COMMANDS.find((c) => c.id === id)?.keys === keys) delete next[id];
    saveOverrides(next);
    onChange(next);
  };
  const rows = useMemo(() => {
    const q = foldText(query.trim());
    return COMMANDS.map((c): Row => ({ id: c.id, label: t(c.label), keys: map[c.id], usual: c.keys, changed: overrides[c.id] !== undefined, clash: map[c.id] !== '' && commandsFor(map[c.id], map).length > 1 })).filter(
      (r) => !q || foldText(r.label).includes(q) || foldText(r.keys).includes(q),
    );
  }, [query, map, overrides, t]); // eslint-disable-line react-hooks/exhaustive-deps

  const menuFor = (at: { x: number; y: number }, list: Row[]) => {
    const r = list[0];
    if (!r) return;
    setMenu({
      ...at,
      items: [
        { label: t('keys.change'), keys: 'Enter', run: () => setRecording(r.id) },
        { label: t('keys.remove'), keys: 'Del', disabled: list.every((x) => x.keys === ''), run: () => list.forEach((x) => set(x.id, '')) },
        SEPARATOR,
        { label: t('keys.reset'), disabled: list.every((x) => !x.changed), run: () => list.forEach((x) => set(x.id, x.usual)) },
      ],
    });
  };

  const columns: Column<Row>[] = [
    { id: 'command', label: t('keys.col.command'), width: 260, fixed: true, sort: (r) => r.label, render: (r) => <span className="ellipsis"><Highlight text={r.label} query={query} /></span> },
    {
      id: 'keys',
      label: t('keys.col.keys'),
      width: 180,
      sort: (r) => r.keys,
      render: (r) =>
        recording === r.id ? (
          <KeyCapture
            onDone={(keys) => {
              setRecording(null);
              if (keys !== null) set(r.id, keys);
            }}
          />
        ) : r.keys ? (
          <span className="kbd-chip">{r.keys}</span>
        ) : (
          <span className="muted-cell">{t('keys.none')}</span>
        ),
    },
    { id: 'source', label: t('keys.col.source'), width: 110, sort: (r) => (r.changed ? 1 : 0), render: (r) => <span className={r.changed ? '' : 'muted-cell'}>{t(r.changed ? 'keys.source.changed' : 'keys.source.usual')}</span> },
    {
      id: 'when',
      label: t('keys.col.clash'),
      width: 200,
      sort: (r) => (r.clash ? 1 : 0),
      render: (r) =>
        r.clash ? (
          <span className="warn ellipsis" title={t('keys.clash')}>
            <Icon name="warning" size={12} /> {t('keys.clash')}
          </span>
        ) : null,
    },
    {
      id: 'tools',
      label: '',
      width: 64,
      fixed: true,
      render: (r) => (
        <>
          <Button variant="flat" size="small" icon="pencil" title={t('keys.change')} onClick={() => setRecording(r.id)} />
          <Button variant="flat" size="small" icon="undo" title={t('keys.reset')} disabled={!r.changed} onClick={() => set(r.id, r.usual)} />
        </>
      ),
    },
  ];

  return (
    <div className="shortcuts">
      <Toolbar label={t('settings.page.shortcuts')} className="inner">
        <FilterBox value={query} onChange={setQuery} placeholder={t('keys.filter')} count={String(rows.length)} />
        <span className="spacer" />
        <Button size="small" icon="undo" disabled={Object.keys(overrides).length === 0} onClick={() => COMMANDS.forEach((c) => overrides[c.id] !== undefined && set(c.id, c.keys))}>
          {t('keys.resetAll')}
        </Button>
      </Toolbar>
      <DataTable
        id="shortcuts"
        label={t('settings.page.shortcuts')}
        rows={rows}
        rowKey={(r) => r.id}
        columns={columns}
        selection={selection}
        onSelection={setSelection}
        onActivate={(r) => setRecording(r.id)}
        rowClass={(r) => (r.clash ? 'clash' : '')}
        onKey={(e, list) => {
          if (e.key === 'Delete' || e.key === 'Backspace') {
            list.forEach((r) => set(r.id, ''));
            return true;
          }
          return false;
        }}
        onMenu={menuFor}
        empty={t('keys.noMatch')}
      />
      {menu && <ContextMenu x={menu.x} y={menu.y} items={menu.items} onClose={() => setMenu(null)} />}
    </div>
  );
}
