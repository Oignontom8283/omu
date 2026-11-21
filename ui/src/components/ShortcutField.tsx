import { comboFromEvent } from '../graph.ts';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';

/** A field that records a keyboard shortcut: click it, then press the keys. */
export function ShortcutField({ value, onChange }: { value: string; onChange: (keys: string) => void }) {
  const t = useT();
  return (
    <div className="shortcut nodrag">
      <input
        className="kit-text code"
        value={value}
        readOnly
        placeholder={t('field.shortcutPlaceholder')}
        onKeyDown={(e) => {
          if (e.key === 'Tab') return; // keep keyboard navigation
          e.preventDefault();
          const combo = comboFromEvent(e.nativeEvent);
          if (combo) onChange(combo);
        }}
      />
      {value && (
        <Button variant="flat" icon="close" title={t('app.close')} onClick={() => onChange('')} />
      )}
    </div>
  );
}
