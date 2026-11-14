import { comboFromEvent } from '../graph.ts';
import { useT } from '../i18n/index.tsx';

/** A field that records a keyboard shortcut: click it, then press the keys. */
export function ShortcutField({ value, onChange }: { value: string; onChange: (keys: string) => void }) {
  const t = useT();
  return (
    <div className="shortcut nodrag">
      <input
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
        <button type="button" className="plain" aria-label={t('app.close')} onClick={() => onChange('')}>
          ×
        </button>
      )}
    </div>
  );
}
