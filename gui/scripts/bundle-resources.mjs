// Files the installers ship besides the programs, listed in src-tauri/tauri.resources.conf.json
// (the list form: it keeps folders, unlike the map form):
// - THIRD-PARTY-LICENSES.html, copied from the repository root.
// - On Windows, which has no git: MinGit (the minimal Git for Windows made for apps that use git)
//   with git-lfs in src-tauri/git/, which furnish finds beside furnish.exe (catalog::git_command).
//   Versions pinned in bundled-git.json, checked by SHA-256. FURNISH_BUNDLE_GIT=1 adds it on
//   other systems too (needs bsdtar), for testing.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const { mingit: MINGIT, lfs: LFS } = JSON.parse(readFileSync(new URL('./bundled-git.json', import.meta.url), 'utf8'));

const tauri = fileURLToPath(new URL('../src-tauri/', import.meta.url));
const config = `${tauri}tauri.resources.conf.json`;
const write = (resources) => writeFileSync(config, JSON.stringify({ bundle: { resources } }, null, 2) + '\n');
copyFileSync(`${tauri}../../THIRD-PARTY-LICENSES.html`, `${tauri}THIRD-PARTY-LICENSES.html`);
if (process.platform !== 'win32' && process.env.FURNISH_BUNDLE_GIT !== '1') {
  write(['THIRD-PARTY-LICENSES.html']);
  process.exit(0);
}

const cache = `${tauri}../../.cache/bundled-git/`;
mkdirSync(cache, { recursive: true });
async function download({ url, sha256 }) {
  const file = cache + url.split('/').pop();
  if (!existsSync(file)) {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`${url}: ${res.status}`);
    writeFileSync(`${file}.part`, Buffer.from(await res.arrayBuffer()));
    renameSync(`${file}.part`, file);
  }
  const actual = createHash('sha256').update(readFileSync(file)).digest('hex');
  if (actual !== sha256) {
    rmSync(file);
    throw new Error(`${url}: SHA-256 ${actual}, expected ${sha256}`);
  }
  return file;
}
// Windows' own tar (bsdtar) extracts zip files; Git Bash's GNU tar on PATH doesn't.
const tar = process.platform === 'win32' ? `${process.env.SystemRoot}\\System32\\tar.exe` : 'bsdtar';
const unzip = (zip, into) => execFileSync(tar, ['-xf', zip, '-C', into]);

const git = `${tauri}git`;
rmSync(git, { recursive: true, force: true });
mkdirSync(git);
unzip(await download(MINGIT), git);
const lfs = `${cache}lfs`;
rmSync(lfs, { recursive: true, force: true });
mkdirSync(lfs);
unzip(await download(LFS), lfs);
// MinGit's git puts ucrt64\bin on PATH, and its gitconfig sets up the LFS filter for git-lfs.
renameSync(`${lfs}/${LFS.exe}`, `${git}/ucrt64/bin/git-lfs.exe`);
write(['THIRD-PARTY-LICENSES.html', 'git/**/*']);
console.log(`bundled git in ${git}`);
