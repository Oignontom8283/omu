import { useEffect, useRef, useState } from 'react';
import { CATEGORIES, KINDS_BY_CATEGORY, defaultKind } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import type { Category, NodeKind } from '../types.ts';

/** The blocks, by family: a solid coloured button per family, and the family's blocks to pick from right under it. */
export function CategoryBar({ onAdd }: { onAdd: (category: Category, kind: NodeKind) => void }) {
  const t = useT();
  const [open, setOpen] = useState<Category | null>(null);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(null);
    };
    const escape = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(null);
    };
    document.addEventListener('pointerdown', away);
    document.addEventListener('keydown', escape);
    return () => {
      document.removeEventListener('pointerdown', away);
      document.removeEventListener('keydown', escape);
    };
  }, [open]);
  return (
    <div className="cats" ref={root}>
      {CATEGORIES.map((c) => (
        <div key={c} className={`cat-wrap cat-${c}`}>
          <button className={`cat${open === c ? ' open' : ''}`} aria-expanded={open === c} title={t(`category.${c}.hint`)} onClick={() => setOpen(open === c ? null : c)}>
            {t(`category.${c}`)}
          </button>
          {open === c && (
            <div className="cat-menu" role="menu">
              {KINDS_BY_CATEGORY[c].map((k) => (
                <button
                  key={k}
                  role="menuitem"
                  onClick={() => {
                    onAdd(c, defaultKind(k));
                    setOpen(null);
                  }}
                >
                  {t(`kind.${k}`)}
                </button>
              ))}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}
