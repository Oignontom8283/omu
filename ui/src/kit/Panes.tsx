import { forwardRef, useEffect, useRef, type ReactNode, type Ref } from 'react';
import { Splitter } from '../components/Workbench.tsx';
import { useT } from '../i18n/index.tsx';
import { Icon } from '../components/Icon.tsx';
import { cleanSize, usePersistent } from './persist.ts';
import { TextInput } from './controls.tsx';

/**
 * A pane of a page, as in a code editor: a thin title bar (its name, then its own buttons on the right) over what it holds, which
 * scrolls on its own.
 */
export function Pane({ title, actions, children, className, label }: { title?: ReactNode; actions?: ReactNode; children: ReactNode; className?: string; label?: string }) {
  return (
    <section className={`pane ${className ?? ''}`} aria-label={label ?? (typeof title === 'string' ? title : undefined)}>
      {(title || actions) && (
        <header className="pane-head">
          <span className="pane-title">{title}</span>
          <span className="pane-actions">{actions}</span>
        </header>
      )}
      <div className="pane-body">{children}</div>
    </section>
  );
}

/**
 * A pane at a side of a page, with the bar that changes its size (dragged, or moved with the arrows; a double click gives the usual
 * size back). Its size is kept in this browser under `id`.
 */
export function SidePane({ id, side, usual, min = 160, max = 900, children }: { id: string; side: 'left' | 'right' | 'bottom'; usual: number; min?: number; max?: number; children: ReactNode }) {
  const t = useT();
  const [size, setSize] = usePersistent(`omu-pane-${id}`, usual, cleanSize(min, max));
  const bar = <Splitter direction={side === 'bottom' ? 'horizontal' : 'vertical'} before={side === 'left'} size={size} min={min} max={max} usual={usual} label={t('dock.resize')} onSize={setSize} />;
  const box = (
    <div className={`side-pane side-${side}`} style={side === 'bottom' ? { height: size } : { width: size }}>
      {children}
    </div>
  );
  return side === 'left' ? (
    <>
      {box}
      {bar}
    </>
  ) : (
    <>
      {bar}
      {box}
    </>
  );
}

/** The bar of buttons and fields at the top of a page or of a pane. */
export function Toolbar({ children, label, className }: { children: ReactNode; label: string; className?: string }) {
  return (
    <div className={`toolbar ${className ?? ''}`} role="toolbar" aria-label={label}>
      {children}
    </div>
  );
}

/** A thin line between two groups of buttons of a toolbar. */
export const ToolSeparator = () => <span className="tool-sep" role="separator" />;

/**
 * The field that filters a list: a magnifier, the text, how many things it lets through, and a cross that empties it (Escape does the
 * same). `focusKey` is a number that, when it changes, puts the cursor in the field (the page's Ctrl+F).
 */
export const FilterBox = forwardRef(function FilterBox(
  { value, onChange, placeholder, count, focusKey }: { value: string; onChange: (text: string) => void; placeholder: string; count?: string; focusKey?: number },
  ref: Ref<HTMLDivElement>,
) {
  const t = useT();
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (focusKey) box.current?.querySelector('input')?.focus();
  }, [focusKey]);
  return (
    <div
      ref={(el) => {
        box.current = el;
        if (typeof ref === 'function') ref(el);
        else if (ref) ref.current = el;
      }}
      className={`filter-box${value ? ' active' : ''}`}
      onKeyDown={(e) => {
        if (e.key === 'Escape' && value) {
          e.stopPropagation();
          onChange('');
        }
      }}
    >
      <Icon name="search" size={13} />
      <TextInput code={false} placeholder={placeholder} title={placeholder} value={value} onChange={onChange} />
      {value && count && <small className="filter-count">{count}</small>}
      {value && (
        <button type="button" className="filter-clear" title={t('settings.clearSearch')} aria-label={t('settings.clearSearch')} onClick={() => onChange('')}>
          <Icon name="close" size={12} />
        </button>
      )}
    </div>
  );
});

/** Without capitals and accents, so that "reseau" finds "Réseau". */
export const foldText = (s: string): string => s.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();

/** A text with the words of a search marked in it (capitals and accents do not count). */
export function Highlight({ text, query }: { text: string; query: string }) {
  const words = foldText(query).split(/\s+/).filter((w) => w.length > 0 && !w.startsWith('@'));
  if (words.length === 0) return <>{text}</>;
  // the folded text keeps one letter per letter of the text for the alphabets the program is written in, so the places match
  const folded = foldText(text);
  if (folded.length !== text.length) return <>{text}</>;
  const marks = new Array<boolean>(text.length).fill(false);
  for (const w of words) for (let i = folded.indexOf(w); i >= 0; i = folded.indexOf(w, i + 1)) marks.fill(true, i, i + w.length);
  const parts: ReactNode[] = [];
  let start = 0;
  for (let i = 1; i <= text.length; i++) {
    if (i === text.length || marks[i] !== marks[start]) {
      const piece = text.slice(start, i);
      parts.push(marks[start] ? <mark key={start}>{piece}</mark> : piece);
      start = i;
    }
  }
  return <>{parts}</>;
}
