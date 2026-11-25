import { useEffect, useRef, useState, type ComponentType } from 'react';
import { NumberField, Select, Slider, Switch, TextInput } from '../kit/controls.tsx';
import { textOf, type SettingControl, type SettingDecl, type SettingValue, type Translate } from './registry.ts';
import { SECTIONS } from './sections.tsx';
import type { SettingsEnv } from './env.ts';

type Of<K extends SettingControl['type']> = Extract<SettingControl, { type: K }>;

/** What every control receives: the setting, what it is worth now, and the way to change it. */
export type ControlProps<K extends SettingControl['type']> = {
  control: Of<K>;
  decl: SettingDecl;
  /** The title as it is shown (it names the control for the people who cannot see the page). */
  label: string;
  value: SettingValue;
  set: (value: SettingValue) => void;
  disabled: boolean;
  t: Translate;
  env: SettingsEnv;
};

function ToggleControl({ value, set, disabled, label }: ControlProps<'toggle'>) {
  return (
    <Switch checked={Boolean(value)} disabled={disabled} title={label} onChange={set} />
  );
}

function ChoiceControl({ control, value, set, disabled, label, t }: ControlProps<'choice'>) {
  const options = typeof control.options === 'function' ? control.options(t) : control.options;
  return <Select className="setting-choice" value={String(value)} label={label} disabled={disabled} options={options} onChange={set} />;
}

function TextControl({ control, value, set, disabled }: ControlProps<'text'>) {
  return <TextInput code={false} value={String(value)} placeholder={control.placeholder} disabled={disabled} onChange={set} />;
}

/** A number is saved a moment after it stops changing (dragging over it changes it many times a second). */
function NumberControl({ control, decl, value, set, disabled, t }: ControlProps<'number'>) {
  const [shown, setShown] = useState<number | null>(null);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  return (
    <NumberField
      value={shown ?? Number(value)}
      min={control.min}
      max={control.max}
      step={control.step}
      whole={control.whole}
      unit={control.unit ? textOf(t, decl.plain, control.unit) : undefined}
      disabled={disabled}
      onChange={(n) => {
        setShown(n);
        window.clearTimeout(timer.current);
        timer.current = window.setTimeout(() => {
          set(n);
          setShown(null);
        }, 400);
      }}
    />
  );
}

function SliderControl({ control, value, set, disabled, label }: ControlProps<'slider'>) {
  return (
    <div className="setting-slider">
      <Slider label={label} min={control.min} max={control.max} step={control.step} value={Number(value)} disabled={disabled} onChange={set} />
      <span className="code">{String(value)}</span>
    </div>
  );
}

/** A block of the page that a component draws, found by its name. */
function SectionControl({ control, env }: ControlProps<'section'>) {
  const Section = SECTIONS[control.component];
  return Section ? <Section env={env} /> : null;
}

/** What an extension draws itself: it is given a place and the value, and may return what cleans up. */
function CustomControl({ control, value, set }: ControlProps<'custom'>) {
  const box = useRef<HTMLDivElement>(null);
  const latest = useRef({ value, set });
  latest.current = { value, set };
  const mount = control.mount;
  useEffect(() => {
    if (!box.current) return;
    const stop = mount(box.current, { value: latest.current.value, set: (v) => latest.current.set(v) });
    return typeof stop === 'function' ? stop : undefined;
  }, [mount]);
  return <div ref={box} className="setting-custom" />;
}

/** The controls, one component per type of setting. A new kind of control is a component added here and a type in `registry.ts`. */
export const CONTROLS: { [K in SettingControl['type']]: ComponentType<ControlProps<K>> } = {
  toggle: ToggleControl,
  choice: ChoiceControl,
  text: TextControl,
  number: NumberControl,
  slider: SliderControl,
  section: SectionControl,
  custom: CustomControl,
};
