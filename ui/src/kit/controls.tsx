import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { Icon, type IconName } from '../components/Icon.tsx';
import { evaluate, written } from './expression.ts';

// The controls of the editor: the program's own components, drawn and handled here, never the browser's. Blocks use them, and
// so can anything that adds a block (see `registry.ts`).

const join = (...names: (string | false | undefined)[]) => names.filter(Boolean).join(' ');

// ---------------------------------------------------------------------------------------------------------- Button

export type ButtonProps = {
  children?: ReactNode;
  icon?: IconName;
  variant?: 'secondary' | 'primary' | 'flat' | 'danger';
  size?: 'normal' | 'small';
  disabled?: boolean;
  title?: string;
  pressed?: boolean;
  /** Takes the whole width of its row. */
  wide?: boolean;
  className?: string;
  onClick?: (e: React.MouseEvent) => void;
};

/** A button: flat, one height, an optional icon. `nodrag` so that a block is not moved by pressing it. */
export function Button({ children, icon, variant = 'secondary', size = 'normal', disabled, title, pressed, wide, className, onClick }: ButtonProps) {
  return (
    <button
      type="button"
      className={join('kit-button nodrag', variant, size, wide && 'wide', pressed && 'pressed', !children && 'icon-only', className)}
      disabled={disabled}
      title={title}
      aria-label={!children ? title : undefined}
      aria-pressed={pressed}
      onClick={onClick}
    >
      {icon && <Icon name={icon} size={size === 'small' ? 12 : 14} />}
      {children}
    </button>
  );
}

// ---------------------------------------------------------------------------------------------------------- TextInput

export type TextInputProps = {
  value: string;
  onChange: (text: string) => void;
  placeholder?: string;
  disabled?: boolean;
  title?: string;
  /** Code font (identifiers, templates, values) rather than the interface's. */
  code?: boolean;
  className?: string;
};

/** A line of text: flat, with a frame that lights up when it has the focus. */
export function TextInput({ value, onChange, placeholder, disabled, title, code = true, className }: TextInputProps) {
  return (
    <input
      className={join('kit-text nodrag', code && 'code', className)}
      value={value}
      placeholder={placeholder}
      disabled={disabled}
      title={title}
      spellCheck={false}
      autoComplete="off"
      onChange={(e) => onChange(e.target.value)}
    />
  );
}

// ---------------------------------------------------------------------------------------------------------- NumberField

export type NumberFieldProps = {
  value: number;
  onChange: (n: number) => void;
  min?: number;
  max?: number;
  step?: number;
  whole?: boolean;
  unit?: string;
  disabled?: boolean;
  title?: string;
  className?: string;
};

/**
 * A number. It is changed by dragging sideways over it (as in a design tool), with the arrows (Shift: ten times more, Alt: ten
 * times less), or by typing: what is typed can be an expression (`2*3`). It never goes past `min` and `max`. Escape gives up.
 */
export function NumberField({ value, onChange, min = -1e9, max = 1e9, step = 1, whole = false, unit, disabled, title, className }: NumberFieldProps) {
  const input = useRef<HTMLInputElement>(null);
  const [editing, setEditing] = useState(false);
  const [typed, setTyped] = useState(written(value, whole));
  const crop = (n: number) => Math.min(max, Math.max(min, whole ? Math.round(n) : n));
  useEffect(() => {
    if (!editing) setTyped(written(value, whole));
  }, [value, whole, editing]);

  const commit = (text: string) => {
    const n = evaluate(text);
    if (n !== null) onChange(crop(n));
    setTyped(written(n === null ? value : crop(n), whole));
  };
  const nudge = (e: KeyboardEvent, direction: 1 | -1) => {
    e.preventDefault();
    const factor = e.shiftKey ? 10 : e.altKey ? 0.1 : 1;
    const next = crop(Math.round((value + direction * step * factor) * 1e6) / 1e6);
    onChange(next);
    setTyped(written(next, whole));
  };

  // pressing and moving sideways changes the value; pressing and letting go types in it
  const press = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (disabled || editing || e.button !== 0) return;
    const start = { x: e.clientX, value };
    let scrubbing = false;
    const target = e.currentTarget;
    const move = (ev: PointerEvent) => {
      const dx = ev.clientX - start.x;
      if (!scrubbing && Math.abs(dx) < 3) return;
      scrubbing = true;
      const factor = ev.shiftKey ? 10 : ev.altKey ? 0.1 : 1;
      onChange(crop(Math.round((start.value + Math.round(dx / 3) * step * factor) * 1e6) / 1e6));
    };
    const stop = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', stop);
      window.removeEventListener('pointercancel', stop);
      target.classList.remove('scrubbing');
      if (!scrubbing) {
        input.current?.focus();
        input.current?.select();
      }
    };
    target.classList.add('scrubbing');
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', stop);
    window.addEventListener('pointercancel', stop);
  };

  return (
    <div className={join('kit-number nodrag', editing && 'editing', disabled && 'disabled', className)} title={title} onPointerDown={press}>
      <input
        ref={input}
        className="code"
        inputMode="decimal"
        value={typed}
        disabled={disabled}
        spellCheck={false}
        onFocus={() => setEditing(true)}
        onBlur={() => {
          setEditing(false);
          commit(typed);
        }}
        onChange={(e) => setTyped(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'ArrowUp') nudge(e, 1);
          else if (e.key === 'ArrowDown') nudge(e, -1);
          else if (e.key === 'Enter') input.current?.blur();
          else if (e.key === 'Escape') {
            setTyped(written(value, whole));
            input.current?.blur();
          }
        }}
      />
      {unit && <small className="unit">{unit}</small>}
    </div>
  );
}

// ---------------------------------------------------------------------------------------------------------- Slider

export type SliderProps = {
  value: number;
  /** The range the slider offers. */
  min: number;
  max: number;
  step?: number;
  onChange: (n: number) => void;
  disabled?: boolean;
  title?: string;
  label?: string;
  className?: string;
};

/** A slider over an advised range; a value outside it (typed, or given by a flow) keeps the thumb at the end, outlined. */
export function Slider({ value, min, max, step = 1, onChange, disabled, title, label, className }: SliderProps) {
  const track = useRef<HTMLDivElement>(null);
  const inside = Math.min(max, Math.max(min, value));
  const at = max > min ? (inside - min) / (max - min) : 0;
  const snap = (n: number) => Math.min(max, Math.max(min, Math.round(n / step) * step));
  const fromPointer = (clientX: number) => {
    const r = track.current?.getBoundingClientRect();
    if (r && r.width > 0) onChange(snap(min + ((clientX - r.left) / r.width) * (max - min)));
  };
  const press = (e: ReactPointerEvent<HTMLDivElement>) => {
    if (disabled || e.button !== 0) return;
    e.currentTarget.focus();
    fromPointer(e.clientX);
    const move = (ev: PointerEvent) => fromPointer(ev.clientX);
    const stop = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', stop);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', stop);
  };
  const key = (e: KeyboardEvent) => {
    const big = e.shiftKey ? 10 : 1;
    const to = (n: number) => {
      e.preventDefault();
      onChange(snap(n));
    };
    if (e.key === 'ArrowRight' || e.key === 'ArrowUp') to(value + step * big);
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowDown') to(value - step * big);
    else if (e.key === 'Home') to(min);
    else if (e.key === 'End') to(max);
  };
  return (
    <div
      className={join('kit-slider nodrag', disabled && 'disabled', value !== inside && 'beyond', className)}
      role="slider"
      tabIndex={disabled ? -1 : 0}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      aria-label={label}
      aria-disabled={disabled}
      title={title}
      onPointerDown={press}
      onKeyDown={key}
    >
      <div ref={track} className="track">
        <div className="fill" style={{ width: `${at * 100}%` }} />
        <div className="thumb" style={{ left: `${at * 100}%` }} />
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------------------------------------- Switch

/** A yes/no, as a switch with the words beside it. */
export function Switch({ checked, onChange, children, disabled, title }: { checked: boolean; onChange: (on: boolean) => void; children?: ReactNode; disabled?: boolean; title?: string }) {
  return (
    <button type="button" role="switch" aria-checked={checked} className={join('kit-switch nodrag', checked && 'on')} disabled={disabled} title={title} onClick={() => onChange(!checked)}>
      <span className="rail">
        <span className="knob" />
      </span>
      {children && <span className="words">{children}</span>}
    </button>
  );
}

// ---------------------------------------------------------------------------------------------------------- Select

export type SelectOption<T extends string = string> = { value: T; label: string; hint?: string };

/**
 * A drop-down with a menu of its own (drawn above everything, so that a block's frame never cuts it): arrows, Home, End, Enter,
 * Escape and typing the first letters of an option all work.
 */
export function Select<T extends string>({ value, options, onChange, disabled, title, label, className }: { value: T; options: SelectOption<T>[]; onChange: (value: T) => void; disabled?: boolean; title?: string; label?: string; className?: string }) {
  const button = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const id = useId();
  const current = options.find((o) => o.value === value);
  const typed = useRef({ text: '', at: 0 });

  const show = () => {
    setActive(Math.max(0, options.findIndex((o) => o.value === value)));
    setOpen(true);
  };
  const pick = (o: SelectOption<T>) => {
    setOpen(false);
    button.current?.focus();
    if (o.value !== value) onChange(o.value);
  };
  const key = (e: KeyboardEvent) => {
    if (!open) {
      if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(e.key)) {
        e.preventDefault();
        show();
      }
      return;
    }
    if (e.key === 'Escape') {
      e.preventDefault();
      setOpen(false);
    } else if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActive((a) => Math.min(options.length - 1, a + 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActive((a) => Math.max(0, a - 1));
    } else if (e.key === 'Home') setActive(0);
    else if (e.key === 'End') setActive(options.length - 1);
    else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      pick(options[active]);
    } else if (e.key.length === 1) {
      // the first letters of an option, typed one after the other
      const now = Date.now();
      typed.current = { text: (now - typed.current.at < 800 ? typed.current.text : '') + e.key.toLowerCase(), at: now };
      const found = options.findIndex((o) => o.label.toLowerCase().startsWith(typed.current.text));
      if (found >= 0) setActive(found);
    }
  };
  return (
    <>
      <button
        ref={button}
        type="button"
        className={join('kit-select nodrag', open && 'open', disabled && 'disabled', className)}
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        aria-label={label}
        disabled={disabled}
        title={title}
        onClick={() => (open ? setOpen(false) : show())}
        onKeyDown={key}
        onBlur={() => window.setTimeout(() => setOpen((o) => (document.activeElement?.closest(`[data-menu="${id}"]`) ? o : false)), 0)}
      >
        <span className="current">{current?.label ?? ''}</span>
        <Icon name="chevron" size={10} />
      </button>
      {open && <Menu id={id} anchor={button.current} options={options} value={value} active={active} onActive={setActive} onPick={pick} onClose={() => setOpen(false)} />}
    </>
  );
}

function Menu<T extends string>({ id, anchor, options, value, active, onActive, onPick, onClose }: { id: string; anchor: HTMLElement | null; options: SelectOption<T>[]; value: T; active: number; onActive: (i: number) => void; onPick: (o: SelectOption<T>) => void; onClose: () => void }) {
  const list = useRef<HTMLUListElement>(null);
  const [place, setPlace] = useState<{ left: number; top: number; width: number } | null>(null);
  useLayoutEffect(() => {
    const r = anchor?.getBoundingClientRect();
    if (!r) return;
    const height = Math.min(options.length * 26 + 8, 280);
    const below = r.bottom + 2 + height <= window.innerHeight;
    setPlace({ left: Math.min(r.left, window.innerWidth - 8 - Math.max(r.width, 160)), top: below ? r.bottom + 2 : Math.max(4, r.top - 2 - height), width: r.width });
  }, [anchor, options.length]);
  useEffect(() => {
    const away = (e: Event) => {
      if (!(e.target as Element).closest(`[data-menu="${id}"]`) && e.target !== anchor && !anchor?.contains(e.target as Node)) onClose();
    };
    window.addEventListener('pointerdown', away, true);
    return () => window.removeEventListener('pointerdown', away, true);
  }, [id, anchor, onClose]);
  useEffect(() => {
    list.current?.children[active]?.scrollIntoView({ block: 'nearest' });
  }, [active]);
  const body = useMemo(() => document.body, []);
  if (!place) return null;
  return createPortal(
    <ul ref={list} id={id} data-menu={id} role="listbox" className="kit-menu" style={{ left: place.left, top: place.top, minWidth: Math.max(place.width, 120) }}>
      {options.map((o, i) => (
        <li key={o.value} role="option" aria-selected={o.value === value} className={join(i === active && 'active', o.value === value && 'chosen')} onPointerMove={() => onActive(i)} onClick={() => onPick(o)}>
          <span className="tick">{o.value === value ? '✓' : ''}</span>
          <span className="label">{o.label}</span>
          {o.hint && <small>{o.hint}</small>}
        </li>
      ))}
    </ul>,
    body,
  );
}

// ---------------------------------------------------------------------------------------------------------- ComboInput

/** A line of text that suggests what could be typed: a list under it, narrowed as the letters come (they only have to appear in order). */
export function ComboInput({ value, options, onChange, placeholder, disabled, code = true }: { value: string; options: string[]; onChange: (text: string) => void; placeholder?: string; disabled?: boolean; code?: boolean }) {
  const field = useRef<HTMLInputElement>(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const id = useId();
  const shown = useMemo(() => {
    const letters = value.toLowerCase().replace(/\s+/g, '');
    const matches = (o: string) => {
      let from = 0;
      for (const c of letters) {
        from = o.toLowerCase().indexOf(c, from) + 1;
        if (from === 0) return false;
      }
      return true;
    };
    return options.filter((o) => o !== value && matches(o)).slice(0, 12);
  }, [options, value]);
  const pick = (o: string) => {
    onChange(o);
    setOpen(false);
  };
  return (
    <>
      <input
        ref={field}
        className={join('kit-text nodrag', code && 'code')}
        value={value}
        placeholder={placeholder}
        disabled={disabled}
        spellCheck={false}
        autoComplete="off"
        role="combobox"
        aria-expanded={open && shown.length > 0}
        aria-controls={id}
        onChange={(e) => {
          onChange(e.target.value);
          setOpen(true);
          setActive(-1);
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => window.setTimeout(() => setOpen(false), 120)}
        onKeyDown={(e) => {
          if (!open || shown.length === 0) return;
          if (e.key === 'ArrowDown') {
            e.preventDefault();
            setActive((a) => Math.min(shown.length - 1, a + 1));
          } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            setActive((a) => Math.max(0, a - 1));
          } else if (e.key === 'Enter' && active >= 0) {
            e.preventDefault();
            pick(shown[active]);
          } else if (e.key === 'Escape') setOpen(false);
        }}
      />
      {open && shown.length > 0 && <Menu id={id} anchor={field.current} options={shown.map((o) => ({ value: o, label: o }))} value={value} active={active} onActive={setActive} onPick={(o) => pick(o.value)} onClose={() => setOpen(false)} />}
    </>
  );
}
