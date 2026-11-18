import { useRef, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';
import { useT } from '../i18n/index.tsx';

/**
 * A small window floating over the editor: it is moved by its title, comes to the front when it is touched and is closed
 * with its cross. It never leaves the screen.
 */
export function FloatingWindow({
  title,
  className = '',
  x,
  y,
  order,
  onMove,
  onFocus,
  onClose,
  children,
}: {
  title: ReactNode;
  className?: string;
  x: number;
  y: number;
  /** Its place in the stack: the higher, the nearer. */
  order: number;
  onMove: (x: number, y: number) => void;
  onFocus: () => void;
  onClose: () => void;
  children: ReactNode;
}) {
  const t = useT();
  const box = useRef<HTMLDivElement>(null);
  const grab = (e: ReactPointerEvent) => {
    if (e.button !== 0 || (e.target as Element).closest('button')) return;
    const from = { x: e.clientX - x, y: e.clientY - y };
    const move = (ev: PointerEvent) => {
      const w = box.current?.offsetWidth ?? 0;
      onMove(Math.max(4, Math.min(window.innerWidth - w - 4, ev.clientX - from.x)), Math.max(4, Math.min(window.innerHeight - 40, ev.clientY - from.y)));
    };
    const stop = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
  };
  return (
    <div ref={box} className={`fwin ${className}`} style={{ left: x, top: y, zIndex: 30 + order }} role="dialog" onPointerDown={onFocus}>
      <div className="fwin-title" onPointerDown={grab}>
        <span className="fwin-name">{title}</span>
        <button className="fwin-close" aria-label={t('app.close')} title={t('app.close')} onClick={onClose}>
          ×
        </button>
      </div>
      <div className="fwin-body">{children}</div>
    </div>
  );
}
