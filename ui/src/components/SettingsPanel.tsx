import { useEffect, useState } from 'react';
import type { api } from '../api.ts';
import { useT } from '../i18n/index.tsx';
import type { Settings } from '../types.ts';

type Props = {
  daemon: ReturnType<typeof api>;
  onClose: () => void;
  notify: (text: string) => void;
};

/** The settings of the program itself (not of one block): for now, whether its icon is in the notification area. */
export function SettingsPanel({ daemon, onClose, notify }: Props) {
  const t = useT();
  const [settings, setSettings] = useState<Settings | null>(null);

  useEffect(() => {
    daemon.settings().then(setSettings).catch((e) => notify(e instanceof Error ? e.message : String(e)));
  }, [daemon, notify]);

  const change = async (patch: Partial<Settings>) => {
    if (!settings) return;
    const next = { ...settings, ...patch };
    setSettings(next);
    try {
      await daemon.setSettings(next);
    } catch (e) {
      notify(e instanceof Error ? e.message : String(e));
      setSettings(settings);
    }
  };

  return (
    <aside className="boards settings" aria-label={t('settings.title')}>
      <header>
        <h2>{t('settings.title')}</h2>
        <span className="spacer" />
        <button className="plain" aria-label={t('app.close')} onClick={onClose}>
          ×
        </button>
      </header>
      {settings && (
        <label className="check" title={t('settings.trayHint')}>
          <input type="checkbox" checked={settings.show_tray_icon} onChange={(e) => void change({ show_tray_icon: e.target.checked })} />
          {t('settings.tray')}
        </label>
      )}
      <small className="muted">{t('settings.trayHint')}</small>
    </aside>
  );
}
