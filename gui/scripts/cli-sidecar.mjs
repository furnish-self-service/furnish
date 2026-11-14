// Build the furnish CLI and put it where Tauri expects the sidecar (src-tauri/binaries/
// furnish-<target triple>), so the app's installers ship it: the furnish service runs it.
// FURNISH_CLI_TARGET builds it for another target, e.g. x86_64-unknown-linux-musl: a static
// binary that runs on any distro (packaging/linux/build.sh uses it).
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const host = execFileSync('rustc', ['-vV']).toString().match(/host: (\S+)/)[1];
const target = process.env.FURNISH_CLI_TARGET;
const ext = process.platform === 'win32' ? '.exe' : '';
const targetDir = process.env.CARGO_TARGET_DIR ?? `${root}target`;
execFileSync('cargo', ['build', '--release', '--bin', 'furnish', ...(target ? ['--target', target] : [])], { cwd: root, stdio: 'inherit' });
mkdirSync(`${root}gui/src-tauri/binaries`, { recursive: true });
// Tauri looks for the host's triple; a static Linux binary stands in for it.
copyFileSync(`${targetDir}/${target ? `${target}/` : ''}release/furnish${ext}`, `${root}gui/src-tauri/binaries/furnish-${host}${ext}`);
