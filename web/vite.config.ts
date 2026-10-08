import { defineConfig } from 'vite';
import preact from '@preact/preset-vite';
import type { Connect, Plugin } from 'vite';

// Reserve the API namespace before Vite's SPA fallback, in dev and preview.
function apiBoundary(): Plugin {
  const install = (server: { middlewares: Connect.Server }) => {
    server.middlewares.use((req, res, next) => {
      const pathname = (req.url ?? '').split('?')[0];
      const isApi = pathname === '/api' || pathname.startsWith('/api/');
      const isSessions = pathname === '/api/sessions' || pathname.startsWith('/api/sessions/');
      if (isApi && !isSessions) {
        res.statusCode = 404;
        res.end();
        return;
      }
      next();
    });
  };
  return { name: 'api-boundary', configureServer: install, configurePreviewServer: install };
}

export default defineConfig({
  plugins: [preact(), apiBoundary()],
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    proxy: {
      '^/api/sessions(?:/|\\?|$)': {
        target: 'http://127.0.0.1:3000',
        ws: true,
        rewrite: (path) => path.replace(/^\/api\/sessions(?=\/|\?|$)/, ''),
      },
    },
  },
  preview: { host: '127.0.0.1', port: 4173, strictPort: true },
});
