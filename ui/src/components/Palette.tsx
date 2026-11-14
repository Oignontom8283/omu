import { CATEGORIES } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import type { Category } from '../types.ts';

/** One button per family of blocks: the block is added with the family's first kind, and its drop-down changes it. */
export function Palette({ onAdd }: { onAdd: (category: Category) => void }) {
  const t = useT();
  return (
    <aside className="palette">
      <h2>{t('app.add')}</h2>
      {CATEGORIES.map((c) => (
        <button key={c} className={`cat-${c}`} onClick={() => onAdd(c)} title={t(`category.${c}.hint`)}>
          <span className="swatch" />
          <span>
            {t(`category.${c}`)}
            <small>{t(`category.${c}.hint`)}</small>
          </span>
        </button>
      ))}
    </aside>
  );
}
