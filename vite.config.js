import { defineConfig } from 'vite';

export default defineConfig(({mode}) => ({
  clearScreen: false,
  server: {
    host: '127.0.0.1', port: 1420, strictPort: true,
    // Native builds produce locked files and HTML reports; neither should
    // trigger WebView reloads or consume the frontend development watcher.
    watch: { ignored: [
      '**/src-tauri/**', '**/crates/**', '**/python/**', '**/legacy/**',
      '**/build/**', '**/dist/**', '**/packaging/**', '**/.artifacts/**',
      '**/.venv/**', '**/.release-secrets/**',
    ] },
  },
  build: { outDir: mode === 'server' ? 'dist-web' : 'dist-ui', emptyOutDir: true, target: 'es2021' },
}));
