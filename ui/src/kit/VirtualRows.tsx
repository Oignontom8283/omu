import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';

/**
 * A long list of rows of one height, of which only the ones in view (and a few around them) exist: a thousand sounds or a million
 * cost the same. The rows are placed by their number, so they keep their place when the list is scrolled.
 */
export function VirtualRows({ count, rowHeight, render, className, label }: { count: number; rowHeight: number; render: (index: number) => ReactNode; className?: string; label?: string }) {
  const box = useRef<HTMLDivElement>(null);
  const [top, setTop] = useState(0);
  const [height, setHeight] = useState(320);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const measure = () => setHeight(el.clientHeight);
    measure();
    const watcher = new ResizeObserver(measure);
    watcher.observe(el);
    return () => watcher.disconnect();
  }, []);
  const around = 6;
  const first = Math.max(0, Math.floor(top / rowHeight) - around);
  const last = Math.min(count, Math.ceil((top + height) / rowHeight) + around);
  const rows: ReactNode[] = [];
  for (let i = first; i < last; i++) {
    rows.push(
      <div key={i} className="vrow" role="row" style={{ top: i * rowHeight, height: rowHeight }}>
        {render(i)}
      </div>,
    );
  }
  return (
    <div ref={box} className={`vrows ${className ?? ''}`} role="rowgroup" aria-label={label} onScroll={(e) => setTop(e.currentTarget.scrollTop)}>
      <div className="vrows-space" style={{ height: count * rowHeight }}>
        {rows}
      </div>
    </div>
  );
}
