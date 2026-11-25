import { useState } from 'react';
import { useT } from '../i18n/index.tsx';
import { COMMANDS, bindings, commandsFor, saveOverrides, type CommandId, type Overrides } from '../keybindings.ts';
import { comboFromEvent } from '../graph.ts';
import { Button } from '../kit/controls.tsx';

/** A button that shows a shortcut and, once pressed, takes the next keys as the new one (Escape gives up, Backspace takes it away). */
function KeyCapture({ value, onChange }: { value: string; onChange: (keys: string) => void }) {
  const t = useT();
  const [listening, setListening] = useState(false);
  return (
    <Button
      className={`key-capture${listening ? ' listening' : ''}`}
      icon="command"
      onClick={() => setListening(true)}
      onBlur={() => setListening(false)}
      onKeyDown={(e) => {
        if (!listening) return;
        if (e.key === 'Tab') return;
        e.preventDefault();
        if (e.key === 'Escape') return setListening(false);
        if (e.key === 'Backspace') {
          onChange('');
          return setListening(false);
        }
        const combo = comboFromEvent(e.nativeEvent);
        if (combo) {
          onChange(combo);
          setListening(false);
        }
      }}
    >
      {listening ? t('keys.press') : value || t('keys.none')}
    </Button>
  );
}

/** Every command and its shortcut, to change: two commands on the same keys are flagged. What is changed is kept in this browser. */
export function Shortcuts({ overrides, onChange }: { overrides: Overrides; onChange: (o: Overrides) => void }) {
  const t = useT();
  const map = bindings(overrides);
  const set = (id: CommandId, keys: string) => {
    const next = { ...overrides, [id]: keys };
    // back to the shipped one: nothing to remember
    if (COMMANDS.find((c) => c.id === id)?.keys === keys) delete next[id];
    saveOverrides(next);
    onChange(next);
  };
  return (
    <div className="shortcuts">
      <div>
        {COMMANDS.map((c) => {
          const clash = map[c.id] !== '' && commandsFor(map[c.id], map).length > 1;
          return (
            <div key={c.id} className={`shortcut-row${clash ? ' clash' : ''}`}>
              <span>{t(c.label)}</span>
              <KeyCapture value={map[c.id]} onChange={(keys) => set(c.id, keys)} />
              <Button variant="flat" size="small" icon="undo" title={t('keys.reset')} disabled={overrides[c.id] === undefined} onClick={() => set(c.id, c.keys)} />
              {clash && <small className="warn">{t('keys.clash')}</small>}
            </div>
          );
        })}
      </div>
    </div>
  );
}
