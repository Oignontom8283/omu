import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';

/**
 * A window floating over the page: it is moved by its title, resized by its corner, comes to the front when it is touched and is
 * closed with its cross. It never leaves the screen. Every window of the program is one of these, a question too.
 *
 * `modal` greys the page behind it and keeps it from being used until the window is closed (Escape closes it, as its cross does).
 */
export function FloatingWindow({
  title,
  className = '',
  x,
  y,
  order,
  modal = false,
  resizable = true,
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
  modal?: boolean;
  /** A corner to drag to change its size (on by default). */
  resizable?: boolean;
  onMove: (x: number, y: number) => void;
  onFocus: () => void;
  onClose: () => void;
  children: ReactNode;
}) {
  const t = useT();
  const box = useRef<HTMLDivElement>(null);
  // the size the person gave it; none until they do (it is as big as what it holds)
  const [size, setSize] = useState<{ w: number; h: number } | null>(null);
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
  const stretch = (e: ReactPointerEvent) => {
    if (e.button !== 0 || !box.current) return;
    e.preventDefault();
    e.stopPropagation();
    const start = { x: e.clientX, y: e.clientY, w: box.current.offsetWidth, h: box.current.offsetHeight };
    const move = (ev: PointerEvent) =>
      setSize({
        w: Math.max(240, Math.min(window.innerWidth - x - 4, start.w + ev.clientX - start.x)),
        h: Math.max(120, Math.min(window.innerHeight - y - 4, start.h + ev.clientY - start.y)),
      });
    const stop = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
  };
  useEffect(() => {
    if (!modal) return;
    const key = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    window.addEventListener('keydown', key);
    return () => window.removeEventListener('keydown', key);
  }, [modal, onClose]);
  const z = modal ? 1000 + order : 30 + order;
  return (
    <>
      {modal && <div className="fwin-backdrop" style={{ zIndex: z - 1 }} />}
      <div
        ref={box}
        className={`fwin ${className}${size ? ' sized' : ''}${modal ? ' modal' : ''}`}
        style={{ left: x, top: y, zIndex: z, ...(size ? { width: size.w, height: size.h } : {}) }}
        role={modal ? 'alertdialog' : 'dialog'}
        aria-modal={modal || undefined}
        onPointerDown={onFocus}
      >
        <div className="fwin-title" onPointerDown={grab}>
          <span className="fwin-name">{title}</span>
          <Button variant="flat" size="small" icon="close" title={t('app.close')} onClick={onClose} />
        </div>
        <div className="fwin-body">{children}</div>
        {resizable && <div className="fwin-grip" onPointerDown={stretch} aria-hidden="true" />}
      </div>
    </>
  );
}
