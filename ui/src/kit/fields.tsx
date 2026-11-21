import { cropTo, portsOf } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import { useEdges } from '@xyflow/react';
import { TemplateEditor, type TemplateValue } from '../components/TemplateEditor.tsx';
import { NumberField, Slider, Switch, TextInput } from './controls.tsx';
import { registerControl, type FieldProps } from './registry.ts';

// The controls of the program that draw the value of an input, registered under `omu.*` (see `registry.ts`).

function Text({ value, title, disabled, write, port }: FieldProps) {
  const t = useT();
  return <TextInput value={String(value)} disabled={disabled} title={title} placeholder={port.placeholder ? t(port.placeholder) : undefined} onChange={write} />;
}

/** A template, written in a code editor: its values coloured, completed after a `$`, and shown with what they are now. */
function Template({ spec, value, title, disabled, write }: FieldProps) {
  const edges = useEdges();
  const values: TemplateValue[] = portsOf(spec)
    .filter((q) => q.name.startsWith('arg'))
    .map((q) => {
      const typed = spec.inputs?.[q.name];
      const linked = edges.some((e) => e.target === spec.id && e.targetHandle === q.name);
      return { name: q.name, shown: linked ? '(from a flow)' : typed === undefined || typed === '' ? '(empty)' : String(typed) };
    });
  return <TemplateEditor value={String(value)} values={values} disabled={disabled} title={title} onChange={write} />;
}

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
registerControl('omu.template', Template);
registerControl('omu.number', number);
registerControl('omu.number-slider', numberSlider);
registerControl('omu.switch', toggle);
