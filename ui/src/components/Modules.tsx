import { useCallback, useEffect, useState } from 'react';
import type { api } from '../api.ts';
import { useT } from '../i18n/index.tsx';
import { moduleStatus } from '../modules.ts';
import { formatBytes } from '../soundboard.ts';
import type { ModuleInfo } from '../types.ts';

type Props = {
  daemon: ReturnType<typeof api>;
  onClose: () => void;
  notify: (text: string) => void;
};

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/** The panel of the optional modules: components that are not part of the program and that people switch on. */
export function Modules({ daemon, onClose, notify }: Props) {
  const t = useT();
  const [modules, setModules] = useState<ModuleInfo[]>([]);
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState<string | null>(null);

  const refresh = useCallback(() => {
    daemon.modules().then(setModules).catch(() => undefined);
  }, [daemon]);

  // a download is followed closely; otherwise the list is looked at now and then
  const downloading = modules.some((m) => m.installing);
  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, downloading ? 500 : 3000);
    return () => clearInterval(timer);
  }, [refresh, downloading]);

  useEffect(() => {
    if (!confirm) return;
    const timer = window.setTimeout(() => setConfirm(null), 4000);
    return () => window.clearTimeout(timer);
  }, [confirm]);

  const run = async (work: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await work();
    } catch (e) {
      notify(message(e));
    } finally {
      setBusy(false);
      refresh();
    }
  };

  return (
    <aside className="boards modules" aria-label={t('modules.title')}>
      <header>
        <h2>{t('modules.title')}</h2>
        <span className="spacer" />
        <button className="plain" aria-label={t('app.close')} onClick={onClose}>
          ×
        </button>
      </header>
      <p className="muted">{t('modules.intro')}</p>
      {modules.map((m) => {
        const state = moduleStatus(m);
        return (
          <section key={m.id} className={`module ${state}`}>
            <h3>
              {m.name} <span className={`pill ${state === 'on' ? 'open' : ''}`}>{t(`modules.state.${state}`)}</span>
            </h3>
            <p className="muted">{m.description}</p>
            {m.available && (
              <p className="muted">
                {t(m.source === 'local' ? 'modules.fromLocal' : 'modules.fromSystem')}
                {m.version ? ` · ${t('modules.version', { version: m.version })}` : ''}
              </p>
            )}
            {m.installing && (
              <div role="status">
                <progress max={m.installing.total || undefined} value={m.installing.total ? m.installing.done : undefined} />
                <small className="muted">
                  {m.installing.total ? `${formatBytes(m.installing.done)} / ${formatBytes(m.installing.total)}` : formatBytes(m.installing.done)}
                </small>
              </div>
            )}
            {m.error && <p className="warn">{m.error}</p>}
            {!m.available && !m.installing && (
              <p className="muted">{m.downloadable ? t('modules.downloadNote', { mb: m.download_mb }) : t('modules.noDownload')}</p>
            )}
            <div className="row">
              {!m.installing && !m.enabled && (m.available || m.downloadable) && (
                <button className="primary" disabled={busy} onClick={() => void run(() => daemon.installModule(m.id))}>
                  {m.available ? t('modules.switchOn') : t('modules.install', { mb: m.download_mb })}
                </button>
              )}
              {m.enabled && !m.installing && (
                <button disabled={busy} onClick={() => void run(() => daemon.enableModule(m.id, false))}>
                  {t('modules.switchOff')}
                </button>
              )}
              {m.source === 'local' && !m.installing && (
                <button
                  className={confirm === `remove:${m.id}` ? 'danger' : ''}
                  disabled={busy}
                  onClick={() => {
                    if (confirm !== `remove:${m.id}`) setConfirm(`remove:${m.id}`);
                    else {
                      setConfirm(null);
                      void run(() => daemon.removeModule(m.id));
                    }
                  }}
                >
                  {confirm === `remove:${m.id}` ? t('soundboards.deleteConfirm') : t('modules.remove')}
                </button>
              )}
            </div>
          </section>
        );
      })}
    </aside>
  );
}
