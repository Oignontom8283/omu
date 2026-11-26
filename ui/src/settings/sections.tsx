import type { ComponentType } from 'react';
import { useT } from '../i18n/index.tsx';
import { Button } from '../kit/controls.tsx';
import { Certificates } from '../components/Certificates.tsx';
import { Colours } from '../components/Colours.tsx';
import { RemoteAccess } from '../components/RemoteAccess.tsx';
import { Shortcuts } from '../components/Shortcuts.tsx';
import { SoundsManager } from '../components/SoundsManager.tsx';
import { getSnapshot, subCategories, textOf } from './registry.ts';
import type { SettingsEnv } from './env.ts';

type SectionProps = { env: SettingsEnv };

function RemoteSection({ env }: SectionProps) {
  return <RemoteAccess daemon={env.daemon} allowed={Boolean(env.settings?.remote_access)} notify={env.notify} />;
}

function CertificatesSection({ env }: SectionProps) {
  return <Certificates daemon={env.daemon} settings={env.settings} notify={env.notify} onFolder={(folder) => void env.change({ certificates_dir: folder || undefined })} />;
}

function SoundsSection({ env }: SectionProps) {
  return <SoundsManager daemon={env.daemon} library={env.library} setLibrary={env.setLibrary} imports={env.imports} notify={env.notify} />;
}

function ShortcutsSection({ env }: SectionProps) {
  return <Shortcuts overrides={env.shortcuts.overrides} onChange={env.shortcuts.onChange} />;
}

/** The extensions that are installed: each one has its own category, with its settings. */
function ExtensionsSection({ env }: SectionProps) {
  const t = useT();
  const state = getSnapshot();
  // the pages under "Extensions" are "Installed" (this one) and one for each extension
  const installed = subCategories(state, 'extensions', t).filter((c) => c.id !== 'installed');
  if (installed.length === 0) return <p className="muted">{t('settings.extensions.none')}</p>;
  return (
    <ul className="extension-list">
      {installed.map((c) => (
        <li key={c.id}>
          <Button variant="flat" onClick={() => env.go(c.id)}>
            {textOf(t, c.plain, c.title)}
          </Button>
          {c.description && <small className="muted">{textOf(t, c.plain, c.description)}</small>}
        </li>
      ))}
    </ul>
  );
}

/** The blocks of the page that a setting of type `section` names. */
export const SECTIONS: Record<string, ComponentType<SectionProps>> = {
  remote: RemoteSection,
  certificates: CertificatesSection,
  sounds: SoundsSection,
  shortcuts: ShortcutsSection,
  colours: Colours,
  extensions: ExtensionsSection,
};
