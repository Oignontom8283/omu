import { useEffect, useState } from 'react';
import type { api } from '../api.ts';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';
import type { RemoteInfo } from '../types.ts';

type Props = {
  daemon: ReturnType<typeof api>;
  /** Whether the person allowed other devices (the setting). */
  allowed: boolean;
  notify: (text: string) => void;
};

/** What the secure server says once other devices are allowed: the pages to open on them, and the certificate to check. */
export function RemoteAccess({ daemon, allowed, notify }: Props) {
  const t = useT();
  const [info, setInfo] = useState<RemoteInfo | null>(null);

  // the server takes a moment to start after the setting is switched on: read until it says what it does
  useEffect(() => {
    if (!allowed) {
      setInfo(null);
      return;
    }
    let live = true;
    const read = () => daemon.remote().then((i) => live && setInfo(i)).catch(() => undefined);
    void read();
    const timer = window.setInterval(read, 1500);
    return () => {
      live = false;
      window.clearInterval(timer);
    };
  }, [daemon, allowed]);

  if (!allowed) return null;
  const copy = async (url: string) => {
    try {
      await navigator.clipboard.writeText(url);
      notify(t('settings.remoteCopied'));
    } catch {
      notify(url);
    }
  };

  return (
    <div className="remote">
      {info?.error && (
        <small className="warn" role="alert">
          {t('settings.remoteError', { error: info.error })}
        </small>
      )}
      {info?.enabled && (
        <>
          <small className="muted">{t('settings.remoteOpen')}</small>
          <ul className="cert-list" aria-label={t('settings.remoteAddresses')}>
            {info.urls.map((url) => (
              <li key={url}>
                <small className="code remote-url">{url.replace(/#token=.*/, '')}</small>
                <Button size="small" onClick={() => void copy(url)}>
                  {t('settings.remoteCopy')}
                </Button>
              </li>
            ))}
          </ul>
          {info.fingerprint && (
            <small className="muted">
              {info.certificate_generated ? t('settings.remoteMade') : t('settings.remoteOwn')} {t('settings.remoteFingerprint')}
              <span className="code remote-url">{info.fingerprint}</span>
            </small>
          )}
          {info.certificate_generated && <small className="muted">{t('settings.remoteWarning')}</small>}
        </>
      )}
    </div>
  );
}
