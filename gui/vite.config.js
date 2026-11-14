import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Tauri expects a fixed dev port and leaves the terminal output to cargo.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  // '..' lets the UI use the repository's logo.svg instead of a copy.
  server: { port: 1420, strictPort: true, fs: { allow: ['..'] } },
  build: { target: 'es2022' },
});
