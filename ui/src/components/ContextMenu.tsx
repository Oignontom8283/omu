import { useEffect, useLayoutEffect, useRef, useState } from 'react';

/** A choice of a menu: its name, what it does, the keys that do the same (shown on the right), and whether it can be chosen now. */
export type MenuAction = { label: string; run: (e: React.MouseEvent | React.KeyboardEvent) => void; danger?: boolean; disabled?: boolean; keys?: string; checked?: boolean };
/** A line between two groups of choices. */
export type MenuSeparator = { separator: true };
export type MenuItem = MenuAction | MenuSeparator;

export const SEPARATOR: MenuSeparator = { separator: true };
const isAction = (item: MenuItem): item is MenuAction => !('separator' in item);

/**
 * The menu of a right click (or of the menu key): at the pointer, never out of the window, closed by a click elsewhere, by Escape or by
 * the choice. The arrows move through the choices, Enter takes one, and a letter goes to the next choice that starts with it.
 */
export function ContextMenu({ x, y, items, onClose }: { x: number; y: number; items: MenuItem[]; onClose: () => void }) {
  const menu = useRef<HTMLUListElement>(null);
  const [at, setAt] = useState({ left: x, top: y });
  // separators at the ends or side by side say nothing
  const shown = items.filter((item, i) => isAction(item) || (i > 0 && i < items.length - 1 && isAction(items[i - 1])));
  useEffect(() => {
    const away = (e: Event) => {
      if (!menu.current?.contains(e.target as Node)) onClose();
    };
    window.addEventListener('pointerdown', away, true);
    window.addEventListener('blur', onClose);
    window.addEventListener('resize', onClose);
    return () => {
      window.removeEventListener('pointerdown', away, true);
      window.removeEventListener('blur', onClose);
      window.removeEventListener('resize', onClose);
    };
  }, [onClose]);
  // placed once its size is known: on the left of the pointer or above it when there is no room on the right or below
  useLayoutEffect(() => {
    const el = menu.current;
    if (!el) return;
    const { width, height } = el.getBoundingClientRect();
    setAt({ left: x + width > window.innerWidth - 4 ? Math.max(4, x - width) : x, top: y + height > window.innerHeight - 4 ? Math.max(4, y - height) : y });
    el.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus();
  }, [x, y]);
  const buttons = () => [...(menu.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])];
  const key = (e: React.KeyboardEvent) => {
    const list = buttons();
    const i = list.indexOf(document.activeElement as HTMLButtonElement);
    const go = (n: number) => list[(n + list.length) % list.length]?.focus();
    if (e.key === 'Escape' || e.key === 'Tab') onClose();
    else if (e.key === 'ArrowDown') go(i + 1);
    else if (e.key === 'ArrowUp') go(i - 1);
    else if (e.key === 'Home') go(0);
    else if (e.key === 'End') go(list.length - 1);
    else if (e.key.length === 1 && /\S/.test(e.key)) {
      const letter = e.key.toLowerCase();
      const next = list.findIndex((b, n) => n > i && b.textContent?.trim().toLowerCase().startsWith(letter));
      const first = list.findIndex((b) => b.textContent?.trim().toLowerCase().startsWith(letter));
      if (next >= 0 || first >= 0) go(next >= 0 ? next : first);
    } else return;
    e.preventDefault();
    e.stopPropagation();
  };
  return (
    <ul ref={menu} className="context-menu" role="menu" style={at} onKeyDown={key} onContextMenu={(e) => e.preventDefault()}>
      {shown.map((item, i) =>
        isAction(item) ? (
          <li key={`${item.label}-${i}`} role="none">
            <button
              role={item.checked === undefined ? 'menuitem' : 'menuitemcheckbox'}
              aria-checked={item.checked}
              className={item.danger ? 'danger' : ''}
              disabled={item.disabled}
              onClick={(e) => {
                onClose();
                item.run(e);
              }}
            >
              <span className="menu-check" aria-hidden>
                {item.checked ? '✓' : ''}
              </span>
              <span className="menu-label">{item.label}</span>
              {item.keys && <kbd className="menu-keys">{item.keys}</kbd>}
            </button>
          </li>
        ) : (
          <li key={`sep-${i}`} role="separator" className="menu-separator" />
        ),
      )}
    </ul>
  );
}
