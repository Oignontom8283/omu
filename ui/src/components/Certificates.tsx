import { useCallback, useEffect, useState } from 'react';
import type { api } from '../api.ts';
import { useT } from '../i18n/index.tsx';
import { Button, TextInput } from '../kit/controls.tsx';
import type { Settings, TrustReport } from '../types.ts';

type Props = {
  daemon: ReturnType<typeof api>;
  settings: Settings | null;
  /** Saves the folder the person chose (empty: the usual one). */
  onFolder: (folder: string) => void;
  notify: (text: string) => void;
};

/** The certificates folder and what the program found in it: who each certificate is about and until when it is valid. */
export function Certificates({ daemon, settings, onFolder, notify }: Props) {
  const t = useT();
  const [report, setReport] = useState<TrustReport | null>(null);
  const [folder, setFolder] = useState('');

  const refresh = useCallback(() => {
    daemon.certificates().then(setReport).catch((e) => notify(e instanceof Error ? e.message : String(e)));
  }, [daemon, notify]);

  // the report is read again whenever the choice changes (mode or folder)
  useEffect(refresh, [refresh, settings?.trust, settings?.certificates_dir]);
  useEffect(() => setFolder(settings?.certificates_dir ?? ''), [settings?.certificates_dir]);

  return (
    <div className="certificates">
      <div className="setting" data-setting="certificates_dir">
        <label className="choice">
          <span>{t('settings.certFolder')}</span>
          <TextInput value={folder} onChange={setFolder} onCommit={(v) => onFolder(v.trim())} placeholder={report?.folder ?? ''} disabled={!settings} />
        </label>
        <small className="muted">{t('settings.certFolderHint')}</small>
      </div>
      {report && !report.folder_exists && <small className="muted">{t('settings.certNoFolder', { folder: report.folder })}</small>}
      {report && report.certificates.length === 0 && report.folder_exists && <small className="muted">{t('settings.certNone')}</small>}
      {report && report.certificates.length > 0 && (
        <ul className="cert-list" aria-label={t('settings.certList')}>
          {report.certificates.map((c, i) => (
            <li key={`${c.file}-${i}`} className={c.expired ? 'expired' : undefined}>
              <strong>{c.subject}</strong>
              <small className="muted">
                {c.file} · {t('settings.certIssuer', { issuer: c.issuer })} · {c.expired ? t('settings.certExpired', { date: c.expires }) : t('settings.certUntil', { date: c.expires })}
              </small>
            </li>
          ))}
        </ul>
      )}
      {report?.problems.map((p, i) => (
        <small key={i} className={p.severity === 'error' ? 'warn' : 'muted'} role={p.severity === 'error' ? 'alert' : undefined}>
          {p.message}
          {p.hint ? ` ${p.hint}` : ''}
        </small>
      ))}
      <Button size="small" onClick={refresh}>
        {t('settings.certRefresh')}
      </Button>
    </div>
  );
}
