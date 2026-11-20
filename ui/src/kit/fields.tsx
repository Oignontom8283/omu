import { cropTo, portsOf } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import { HighlightedInput } from '../components/HighlightedInput.tsx';
import { NumberField, Slider, Switch, TextInput } from './controls.tsx';
import { registerControl, type FieldProps } from './registry.ts';

// The controls of the program that draw the value of an input, registered under `omu.*` (see `registry.ts`).

function Text({ value, title, disabled, write, port }: FieldProps) {
  const t = useT();
  return <TextInput value={String(value)} disabled={disabled} title={title} placeholder={port.placeholder ? t(port.placeholder) : undefined} onChange={write} />;
}

/** A template: the text with its values coloured as in a code editor. */
const template = ({ spec, value, title, disabled, write }: FieldProps) => {
  const arguments_ = portsOf(spec).filter((q) => q.name.startsWith('arg')).length;
  const known = (name: string) => /^(arg)?\d+$/.test(name) && Number(name.replace('arg', '')) <= arguments_;
  return <HighlightedInput value={String(value)} disabled={disabled} title={title} known={known} onChange={write} />;
};

const number = ({ port, value, title, disabled, write }: FieldProps) => (
  <NumberField value={Number(value) || 0} whole={port.flow === 'int'} min={port.limits?.min} max={port.limits?.max} step={port.step ?? (port.flow === 'int' ? 1 : 0.5)} unit={port.unit} title={title} disabled={disabled} onChange={write} />
);

/** A number with a slider over the advised range, and the number beside it for what lies beyond. */
const numberSlider = ({ port, value, title, disabled, write }: FieldProps) => {
  const l = port.limits;
  const n = Number(value) || 0;
  const whole = port.flow === 'int';
  return (
    <span className="field-pair" title={title}>
      {l && <Slider value={n} min={l.softMin} max={l.softMax} step={whole ? 1 : l.softMax - l.softMin > 50 ? 1 : 0.5} label={port.name} disabled={disabled} onChange={(v) => write(cropTo(l, v))} />}
      <NumberField value={n} whole={whole} min={l?.min} max={l?.max} step={port.step ?? (whole ? 1 : 0.5)} unit={port.unit} disabled={disabled} onChange={write} />
    </span>
  );
};

const toggle = ({ value, title, disabled, write }: FieldProps) => <Switch checked={Boolean(value)} disabled={disabled} title={title} onChange={write} />;

registerControl('omu.text', Text);
registerControl('omu.code', Text);
registerControl('omu.template', template);
registerControl('omu.number', number);
registerControl('omu.number-slider', numberSlider);
registerControl('omu.switch', toggle);
