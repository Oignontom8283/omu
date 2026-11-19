import { useEffect, useRef } from 'react';

export type MenuItem = { label: string; run: () => void; danger?: boolean };

/** The menu of a right click: at the pointer, closed by a click elsewhere, by Escape or by the choice. */
export function ContextMenu({ x, y, items, onClose }: { x: number; y: number; items: MenuItem[]; onClose: () => void }) {
  const menu = useRef<HTMLUListElement>(null);
  useEffect(() => {
    const away = (e: Event) => {
      if (!menu.current?.contains(e.target as Node)) onClose();
    };
    const key = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('pointerdown', away, true);
    window.addEventListener('keydown', key);
    window.addEventListener('blur', onClose);
    return () => {
      window.removeEventListener('pointerdown', away, true);
      window.removeEventListener('keydown', key);
      window.removeEventListener('blur', onClose);
    };
  }, [onClose]);
  // never out of the window
  const left = Math.min(x, window.innerWidth - 220);
  const top = Math.min(y, window.innerHeight - items.length * 32 - 16);
  return (
    <ul ref={menu} className="context-menu" role="menu" style={{ left, top }}>
      {items.map((item) => (
        <li key={item.label} role="none">
          <button
            role="menuitem"
            className={item.danger ? 'danger' : ''}
            onClick={() => {
              onClose();
              item.run();
            }}
          >
            {item.label}
          </button>
        </li>
      ))}
    </ul>
  );
}
