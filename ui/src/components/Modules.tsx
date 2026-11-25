import { useCallback, useEffect, useState } from 'react';
import type { api } from '../api.ts';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';
import { ask } from '../kit/dialogs.tsx';
import { moduleDocument, moduleStatus, moduleText } from '../modules.ts';
import { formatBytes } from '../soundboard.ts';
import type { ModuleInfo } from '../types.ts';
import { useInfoWindows } from './InfoWindows.tsx';

type Props = {
  daemon: ReturnType<typeof api>;
  notify: (text: string) => void;
};

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));

/**
 * The page of the optional modules: components that are not part of the program and that people switch on (nothing is downloaded unless they
 * ask). Each module has its name, where it stands, what it is for, its actions, and an "i" that opens its documentation in a floating window.
 */
export function Modules({ daemon, notify }: Props) {
  const t = useT();
  const [modules, setModules] = useState<ModuleInfo[] | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(() => {
    daemon.modules().then(setModules).catch(() => undefined);
  }, [daemon]);

  // a download is followed closely; otherwise the list is looked at now and then
  const downloading = modules?.some((m) => m.installing) ?? false;
  useEffect(() => {
    refresh();
    const timer = setInterval(refresh, downloading ? 500 : 3000);
    return () => clearInterval(timer);
  }, [refresh, downloading]);

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

  const info = useInfoWindows((key) => {
    const m = modules?.find((x) => `module:${x.id}` === key);
    return m ? { source: moduleDocument(m, t) } : null;
  }, {});

  return (
    <div className="page modules-page" aria-label={t('modules.title')}>
      <div className="page-inner">
        <header className="page-head">
          <div className="page-title">
            <h1>{t('modules.title')}</h1>
          </div>
          <p className="page-lead">{t('modules.intro')}</p>
        </header>
        {modules && modules.length === 0 && <p className="muted">{t('modules.none')}</p>}
        {modules?.map((m) => {
          const state = moduleStatus(m);
          const name = moduleText(m, 'name', t);
          return (
            <section key={m.id} className={`module-item ${state}`} aria-label={name}>
              <div className="setting-head">
                <h3 className="setting-title">{name}</h3>
                <span className={`pill ${state === 'on' ? 'open' : ''}`}>{t(`modules.state.${state}`)}</span>
                <Button variant="flat" size="small" icon="help" title={t('settings.about', { name })} onClick={(e) => info.open(`module:${m.id}`, { x: e.clientX + 12, y: e.clientY - 8 })} />
              </div>
              <p className="setting-desc">{moduleText(m, 'description', t)}</p>
              {m.available && (
                <p className="muted module-meta">
                  {t(m.source === 'local' ? 'modules.fromLocal' : 'modules.fromSystem')}
                  {m.version ? ` · ${t('modules.version', { version: m.version })}` : ''}
                </p>
              )}
              {m.installing && (
                <div role="status" className="module-progress">
                  <progress max={m.installing.total || undefined} value={m.installing.total ? m.installing.done : undefined} />
                  <small className="muted">{m.installing.total ? `${formatBytes(m.installing.done)} / ${formatBytes(m.installing.total)}` : formatBytes(m.installing.done)}</small>
                </div>
              )}
              {m.error && (
                <p className="warn" role="alert">
                  {m.error}
                </p>
              )}
              {m.supported === false && <p className="warn">{t('modules.notSupported')}</p>}
              {!m.available && !m.installing && m.supported !== false && <p className="muted">{m.downloadable ? t('modules.downloadNote', { mb: m.download_mb }) : t('modules.noDownload')}</p>}
              <div className="row">
                {m.installing && (
                  <Button icon="close" disabled={busy} onClick={() => void run(() => daemon.cancelModule(m.id))}>
                    {t('modules.cancel')}
                  </Button>
                )}
                {!m.installing && !m.enabled && m.supported !== false && (m.available || m.downloadable) && (
                  <Button variant="primary" icon={m.available ? 'power' : 'plus'} disabled={busy} onClick={() => void run(() => daemon.installModule(m.id))}>
                    {m.available ? t('modules.switchOn') : t('modules.install', { mb: m.download_mb })}
                  </Button>
                )}
                {m.enabled && !m.installing && (
                  <Button icon="power" disabled={busy} onClick={() => void run(() => daemon.enableModule(m.id, false))}>
                    {t('modules.switchOff')}
                  </Button>
                )}
                {m.source === 'local' && !m.installing && (
                  <Button
                    icon="trash"
                    disabled={busy}
                    title={t('dialog.shiftHint')}
                    onClick={(e) => {
                      void ask({ title: t('dialog.removeModule.title', { name }), message: t('dialog.removeModule.message') }, e).then((yes) => (yes ? run(() => daemon.removeModule(m.id)) : undefined));
                    }}
                  >
                    {t('modules.remove')}
                  </Button>
                )}
              </div>
            </section>
          );
        })}
      </div>
      {info.windows}
    </div>
  );
}
