import { useEffect, useRef } from 'react';
import { useEditor } from '../context.ts';
import { comboFromEvent } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';

/** A field that records a keyboard shortcut: click it, then press the keys. */
export function ShortcutField({ value, onChange }: { value: string; onChange: (keys: string) => void }) {
  const t = useT();
  const { pauseHotkeys } = useEditor();
  // the global shortcuts are let go of while the field has the focus (a key that is in use would never reach it), and always taken back
  const listening = useRef(false);
  const pause = useRef(pauseHotkeys);
  pause.current = pauseHotkeys;
  const listen = (on: boolean) => {
    if (on || listening.current) pause.current(on);
    listening.current = on;
  };
  // the field goes away while it has the focus (the block is removed...): the shortcuts come back
  useEffect(
    () => () => {
      if (listening.current) pause.current(false);
    },
    [],
  );
  return (
    <div className="shortcut nodrag">
      <input
        className="kit-text code"
        value={value}
        readOnly
        onFocus={() => listen(true)}
        onBlur={() => listen(false)}
        placeholder={t('field.shortcutPlaceholder')}
        onKeyDown={(e) => {
          if (e.key === 'Tab') return; // keep keyboard navigation
          e.preventDefault();
          const combo = comboFromEvent(e.nativeEvent);
          if (combo) {
            onChange(combo);
            pauseHotkeys(true); // the pause goes on while keys are being recorded
          }
        }}
      />
      {value && (
        <Button variant="flat" icon="close" title={t('app.close')} onClick={() => onChange('')} />
      )}
    </div>
  );
}
