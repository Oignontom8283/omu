import { useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';
import type { Board } from '../types.ts';
import { Icon } from './Icon.tsx';

const DRAG_TYPE = 'application/x-omu-board';

/**
 * The soundboards as a row of tabs along the top of the page: a click opens one, a double click or F2 renames it in place, it is dragged
 * to another place (or moved with Alt and the arrows), the arrows go from one to the next, Delete removes it, and a right click gives the
 * rest. A dot says which ones are in the running graph. What is given as `tools` sits at the right end of the row.
 */
export function BoardTabs({
  boards,
  current,
  live,
  renaming,
  onPick,
  onStartRename,
  onRename,
  onMove,
  onRemove,
  onAdd,
  onMenu,
  tools,
}: {
  boards: Board[];
  current: string | undefined;
  /** The soundboards that are in the running graph. */
  live: (id: string) => boolean;
  renaming: string | null;
  onPick: (id: string) => void;
  onStartRename: (id: string) => void;
  /** The new name, or null when the renaming is given up. */
  onRename: (id: string, name: string | null) => void;
  /** Puts the soundboard at this place among them all. */
  onMove: (id: string, to: number) => void;
  onRemove: (board: Board, e: { shiftKey?: boolean }) => void;
  onAdd: () => void;
  /** A right click on a tab (its soundboard) or beside them (none). */
  onMenu: (at: { x: number; y: number }, board: Board | null) => void;
  tools?: ReactNode;
}) {
  const t = useT();
  const list = useRef<HTMLDivElement>(null);
  // where a dragged tab would land: before the tab of this index
  const [dropAt, setDropAt] = useState<number | null>(null);
  const focusTab = (id: string) => window.setTimeout(() => list.current?.querySelector<HTMLElement>(`[data-board="${CSS.escape(id)}"]`)?.focus(), 0);

  const keys = (e: KeyboardEvent, b: Board, i: number) => {
    const step = e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowRight' ? 1 : 0;
    if (step && e.altKey) {
      onMove(b.id, Math.max(0, Math.min(boards.length - 1, i + step)));
      focusTab(b.id);
    } else if (step || e.key === 'Home' || e.key === 'End') {
      const to = e.key === 'Home' ? 0 : e.key === 'End' ? boards.length - 1 : (i + step + boards.length) % boards.length;
      onPick(boards[to].id);
      focusTab(boards[to].id);
    } else if (e.key === 'F2') onStartRename(b.id);
    else if (e.key === 'Delete') onRemove(b, e);
    else if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
      onMenu({ x: r.left + 12, y: r.bottom }, b);
    } else return;
    e.preventDefault();
  };

  return (
    <nav
      className="board-tabs"
      aria-label={t('soundboards.tabs')}
      onContextMenu={(e) => {
        if (e.target !== e.currentTarget && (e.target as HTMLElement).closest('.board-tab, button, input')) return;
        e.preventDefault();
        onMenu({ x: e.clientX, y: e.clientY }, null);
      }}
    >
      <div
        ref={list}
        className="board-tab-list"
        role="tablist"
        aria-label={t('soundboards.tabs')}
        onWheel={(e) => {
          // the wheel scrolls a long row of tabs sideways
          if (e.deltaY !== 0 && list.current) list.current.scrollLeft += e.deltaY;
        }}
        onDragLeave={(e) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropAt(null);
        }}
      >
        {boards.map((b, i) => {
          const on = b.id === current;
          const before = dropAt === i;
          const after = dropAt === boards.length && i === boards.length - 1;
          return (
            <div
              key={b.id}
              className={`board-tab${on ? ' on' : ''}${before ? ' drop-before' : ''}${after ? ' drop-after' : ''}`}
              role="presentation"
              draggable={renaming !== b.id}
              onDragStart={(e) => {
                e.dataTransfer.setData(DRAG_TYPE, b.id);
                e.dataTransfer.effectAllowed = 'move';
              }}
              onDragOver={(e) => {
                if (!e.dataTransfer.types.includes(DRAG_TYPE)) return;
                e.preventDefault();
                e.stopPropagation();
                const r = e.currentTarget.getBoundingClientRect();
                setDropAt(e.clientX < r.left + r.width / 2 ? i : i + 1);
              }}
              onDrop={(e) => {
                const id = e.dataTransfer.getData(DRAG_TYPE);
                if (!id) return;
                e.preventDefault();
                e.stopPropagation();
                const from = boards.findIndex((x) => x.id === id);
                const to = dropAt ?? i;
                setDropAt(null);
                // the place is counted among the others, once the dragged one is taken out
                if (from >= 0) onMove(id, to > from ? to - 1 : to);
              }}
              onDragEnd={() => setDropAt(null)}
              onContextMenu={(e) => {
                e.preventDefault();
                onPick(b.id);
                onMenu({ x: e.clientX, y: e.clientY }, b);
              }}
            >
              {renaming === b.id ? (
                <input
                  className="board-tab-input"
                  autoFocus
                  defaultValue={b.name}
                  aria-label={t('soundboards.rename')}
                  onFocus={(e) => e.currentTarget.select()}
                  onBlur={(e) => onRename(b.id, e.currentTarget.value)}
                  onKeyDown={(e) => {
                    e.stopPropagation();
                    if (e.key === 'Enter') e.currentTarget.blur();
                    if (e.key === 'Escape') {
                      onRename(b.id, null);
                      focusTab(b.id);
                    }
                  }}
                />
              ) : (
                <button
                  type="button"
                  role="tab"
                  data-board={b.id}
                  aria-selected={on}
                  tabIndex={on ? 0 : -1}
                  title={`${live(b.id) ? t('soundboards.live') : t('soundboards.notLive')} — ${t('soundboards.tabHint')}`}
                  onClick={() => onPick(b.id)}
                  onDoubleClick={() => onStartRename(b.id)}
                  onKeyDown={(e) => keys(e, b, i)}
                >
                  <span className={`state-dot${live(b.id) ? ' on' : ''}`} />
                  <span className="board-tab-name">{b.name}</span>
                  <span className="board-tab-count">{b.pads.length}</span>
                </button>
              )}
              {on && renaming !== b.id && (
                <button type="button" className="tab-x" tabIndex={-1} title={`${t('soundboards.deleteBoard')} — ${t('dialog.shiftHint')}`} aria-label={t('soundboards.deleteBoard')} onClick={(e) => onRemove(b, e)}>
                  <Icon name="close" size={12} />
                </button>
              )}
            </div>
          );
        })}
        <Button variant="flat" size="small" icon="plus" className="board-tab-add" title={t('soundboards.newBoard')} onClick={onAdd} />
      </div>
      {tools && <div className="board-tab-tools">{tools}</div>}
    </nav>
  );
}
