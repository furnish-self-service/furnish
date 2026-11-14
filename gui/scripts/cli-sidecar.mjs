// Build the furnish CLI and put it where Tauri expects the sidecar (src-tauri/binaries/
// furnish-<target triple>), so the app's installers ship it: the furnish service runs it.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const triple = execFileSync('rustc', ['-vV']).toString().match(/host: (\S+)/)[1];
const ext = process.platform === 'win32' ? '.exe' : '';
execFileSync('cargo', ['build', '--release', '--bin', 'furnish'], { cwd: root, stdio: 'inherit' });
mkdirSync(`${root}gui/src-tauri/binaries`, { recursive: true });
copyFileSync(`${root}target/release/furnish${ext}`, `${root}gui/src-tauri/binaries/furnish-${triple}${ext}`);
