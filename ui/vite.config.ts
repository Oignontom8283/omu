import { defineConfig, type ProxyOptions } from 'vite';
import type { ClientRequest } from 'node:http';
import react from '@vitejs/plugin-react';
import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

// In development the panel is served by Vite and talks to a running daemon: find it through the discovery file the
// daemon writes (the same one the command line reads), proxy /rpc, /ws and /sounds to it, and pass the token to the page.
function daemonDiscovery(): { address: string; token: string } | null {
  const dir =
    process.env.OMU_DATA_DIR ??
    (process.platform === 'win32'
      ? join(process.env.APPDATA ?? join(homedir(), 'AppData', 'Roaming'), 'Omu')
      : process.platform === 'darwin'
        ? join(homedir(), 'Library', 'Application Support', 'Omu')
        : join(process.env.XDG_DATA_HOME ?? join(homedir(), '.local', 'share'), 'Omu'));
  // the daemon publishes where it is in the `run` folder (an earlier version wrote it at the root)
  const file = [join(dir, 'run', 'daemon.json'), join(dir, 'daemon.json')].find((f) => existsSync(f));
  if (!file) return null;
  try {
    return JSON.parse(readFileSync(file, 'utf8'));
  } catch {
    return null;
  }
}

// The product's name and the usual memory of the sounds come from branding.toml (one source of truth for every build target).
const branding = readFileSync(join(import.meta.dirname, '..', 'branding.toml'), 'utf8');
function brandingValue(key: string, pattern: string): string {
  const m = branding.match(new RegExp(`^${key}\\s*=\\s*${pattern}`, 'm'));
  if (!m) throw new Error(`branding.toml: ${key} not found`);
  return m[1];
}

const product = brandingValue('display_name', '"([^"]*)"');
const soundCacheMb = Number(brandingValue('sound_cache_mb', '(\\d+)'));
const daemon = daemonDiscovery();
const target = `http://${daemon?.address ?? '127.0.0.1:7650'}`;

// The daemon refuses browser requests whose Origin is not its own: present the proxy as the daemon itself.
const asDaemonOrigin: NonNullable<ProxyOptions['configure']> = (proxy) => {
  const rewrite = (req: ClientRequest) => req.setHeader('origin', target);
  proxy.on('proxyReq', rewrite);
  proxy.on('proxyReqWs', rewrite);
};

export default defineConfig(({ command }) => ({
  plugins: [
    react(),
    { name: 'product-name', transformIndexHtml: (html: string) => html.replaceAll('%PRODUCT_NAME%', product) },
  ],
  define: {
    // the key of the running daemon is for the development server only: a built panel is embedded in the daemon and served to
    // anyone, so it must never carry a key
    __OMU_DEV_TOKEN__: JSON.stringify(command === 'serve' ? (daemon?.token ?? '') : ''),
    __PRODUCT_NAME__: JSON.stringify(product),
    __SOUND_CACHE_MB__: JSON.stringify(soundCacheMb),
  },
  server: {
    port: 5173,
    proxy: {
      '/rpc': { target, changeOrigin: true, configure: asDaemonOrigin },
      '/ws': { target, changeOrigin: true, ws: true, configure: asDaemonOrigin },
      // a browser sends the sound files it imports to /sounds
      '/sounds': { target, changeOrigin: true, configure: asDaemonOrigin },
    },
  },
  build: { outDir: 'dist', emptyOutDir: true, sourcemap: false },
}));
