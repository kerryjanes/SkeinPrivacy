import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Tauri serves the dev frontend on a fixed port and bundles the built assets.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: 'es2022', outDir: 'dist' },
});
