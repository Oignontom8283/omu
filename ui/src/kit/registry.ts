import type { ComponentType } from 'react';
import type { PortMeta } from '../graph.ts';
import type { InputValue, NodeSpec } from '../types.ts';

/**
 * What a field that shows the value of an input receives. A block says which control draws its input by the name of a
 * registered control (`control: 'omu.number'`); the controls of the program are registered under `omu.*`, and whatever adds
 * blocks can register controls of its own under its own names (or use the program's: they are all in `kit`).
 */
export type FieldProps = {
  spec: NodeSpec;
  port: PortMeta;
  value: InputValue;
  /** A flow is linked to the input: it wins, so the field is only shown. */
  disabled: boolean;
  title?: string;
  /** Types a new value in the input. */
  write: (value: InputValue) => void;
};

const controls = new Map<string, ComponentType<FieldProps>>();

/** Makes a control available to the blocks under `name` (a later registration under the same name replaces the first). */
export function registerControl(name: string, component: ComponentType<FieldProps>): void {
  controls.set(name, component);
}

/** The control registered under `name`, if any. */
export function controlNamed(name: string): ComponentType<FieldProps> | undefined {
  return controls.get(name);
}

/** The names of the controls that exist. */
export function controlNames(): string[] {
  return [...controls.keys()].sort();
}

/** The control a port is drawn with when its definition names none: by the sort of value it takes. */
export function defaultControl(port: PortMeta): string {
  if (port.flow === 'text') return port.name === 'template' ? 'omu.template' : 'omu.text';
  if (port.flow === 'condition') return 'omu.switch';
  return port.limits && (port.limits.softMin > port.limits.min || port.limits.softMax < port.limits.max) ? 'omu.number-slider' : 'omu.number';
}
