import { defineConfig, type ProxyOptions } from 'vite';
import type { ClientRequest } from 'node:http';
import react from '@vitejs/plugin-react';
import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

// In development the panel is served by Vite and talks to a running daemon: find it through the discovery file the
// daemon writes (the same one the command line reads), proxy /rpc and /ws to it, and pass the token to the page.
function daemonDiscovery(): { address: string; token: string } | null {
  const dir =
    process.env.OMU_DATA_DIR ??
    (process.platform === 'win32'
      ? join(process.env.APPDATA ?? join(homedir(), 'AppData', 'Roaming'), 'Omu')
      : process.platform === 'darwin'
        ? join(homedir(), 'Library', 'Application Support', 'Omu')
        : join(process.env.XDG_DATA_HOME ?? join(homedir(), '.local', 'share'), 'Omu'));
  const file = join(dir, 'daemon.json');
  if (!existsSync(file)) return null;
  try {
    return JSON.parse(readFileSync(file, 'utf8'));
  } catch {
    return null;
  }
}

// The product's name comes from branding.toml (one source of truth for every build target).
function productName(): string {
  const text = readFileSync(join(import.meta.dirname, '..', 'branding.toml'), 'utf8');
  const m = text.match(/^display_name\s*=\s*"([^"]*)"/m);
  if (!m) throw new Error('branding.toml: display_name not found');
  return m[1];
}

const product = productName();
const daemon = daemonDiscovery();
const target = `http://${daemon?.address ?? '127.0.0.1:7650'}`;

// The daemon refuses browser requests whose Origin is not its own: present the proxy as the daemon itself.
const asDaemonOrigin: NonNullable<ProxyOptions['configure']> = (proxy) => {
  const rewrite = (req: ClientRequest) => req.setHeader('origin', target);
  proxy.on('proxyReq', rewrite);
  proxy.on('proxyReqWs', rewrite);
};

export default defineConfig({
  plugins: [
    react(),
    { name: 'product-name', transformIndexHtml: (html: string) => html.replaceAll('%PRODUCT_NAME%', product) },
  ],
  define: {
    __OMU_DEV_TOKEN__: JSON.stringify(daemon?.token ?? ''),
    __PRODUCT_NAME__: JSON.stringify(product),
  },
  server: {
    port: 5173,
    proxy: {
      '/rpc': { target, changeOrigin: true, configure: asDaemonOrigin },
      '/ws': { target, changeOrigin: true, ws: true, configure: asDaemonOrigin },
    },
  },
  build: { outDir: 'dist', emptyOutDir: true, sourcemap: false },
});
