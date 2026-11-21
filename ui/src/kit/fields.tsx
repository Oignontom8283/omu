import { cropTo, portsOf } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import { useEdges } from '@xyflow/react';
import type { CodeLanguage, CodeName } from '../components/CodeEditor.tsx';
import { CodeInput } from './code.tsx';
import { NumberField, Slider, Switch } from './controls.tsx';
import { registerControl, type FieldProps } from './registry.ts';

// The controls of the program that draw the value of an input, registered under `omu.*` (see `registry.ts`).

/**
 * A text of a block, written like code: coloured by what it says, completed with the names the block has (`names` tells which of
 * its inputs those are), brackets closed, and a button that opens it in a window of its own.
 */
function code(language: CodeLanguage, names?: (portName: string) => boolean) {
  return function CodeField({ spec, port, value, title, disabled, write }: FieldProps) {
    const t = useT();
    const edges = useEdges();
    const known: CodeName[] = names
      ? portsOf(spec)
          .filter((q) => q.dir === 'in' && !q.param && names(q.name))
          .map((q) => {
            const typed = spec.inputs?.[q.name];
            const linked = edges.some((e) => e.target === spec.id && e.targetHandle === q.name);
            return { name: q.name, shown: linked ? t('code.fromFlow') : typed === undefined || typed === '' ? t('code.empty') : String(typed) };
          })
      : [];
    const label = `${t(`kind.${spec.type}`)} · ${t(port.label, { n: port.n ?? '' }).trim()}`;
    return <CodeInput value={String(value)} onChange={write} language={language} names={known} label={label} disabled={disabled} title={title} placeholder={port.placeholder ? t(port.placeholder) : undefined} />;
  };
}

const Text = code('plain');
/** A template: its values coloured, completed after a `$`, and shown with what they are now. */
const Template = code('template', (name) => name.startsWith('arg'));
/** The expression of a calculation or a test: its inputs `a`, `b`... coloured, completed with the functions, and shown with what they are now. */
const Expression = code('expression', () => true);
/** The test of a block that tests texts: its inputs `a`, `b`... coloured, completed with the functions of a text, and shown with what they are now. */
const TextTest = code('test', () => true);

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
registerControl('omu.expression', Expression);
registerControl('omu.test', TextTest);
registerControl('omu.number', number);
registerControl('omu.number-slider', numberSlider);
registerControl('omu.switch', toggle);
