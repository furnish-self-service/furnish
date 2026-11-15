// Windows has no git, and furnish needs it for collections: the installer ships MinGit (the
// minimal Git for Windows made for apps that use git) with git-lfs in src-tauri/git/, which
// furnish finds beside furnish.exe (catalog::git_command). Pinned versions, checked by SHA-256.
// Writes src-tauri/tauri.git.conf.json, which adds the folder to the installer on Windows and is
// empty elsewhere. FURNISH_BUNDLE_GIT=1 does it on other systems too (needs bsdtar), for testing.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const MINGIT = {
  url: 'https://github.com/git-for-windows/git/releases/download/v2.56.1.windows.1/MinGit-2.56.1-64-bit.zip',
  sha256: '4a92850b6e917da4fd010669b614b55dccf5b0ec333a534cc4be4700dfefdafd',
};
const LFS = {
  url: 'https://github.com/git-lfs/git-lfs/releases/download/v3.8.1/git-lfs-windows-amd64-v3.8.1.zip',
  sha256: '5bb771c12b3dacc535e912d0270a010c209d4da2e63819c5918e3547c2defed9',
  exe: 'git-lfs-3.8.1/git-lfs.exe',
};

const tauri = fileURLToPath(new URL('../src-tauri/', import.meta.url));
const config = `${tauri}tauri.git.conf.json`;
if (process.platform !== 'win32' && process.env.FURNISH_BUNDLE_GIT !== '1') {
  writeFileSync(config, '{}\n');
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
writeFileSync(config, JSON.stringify({ bundle: { resources: ['git/**/*'] } }, null, 2) + '\n');
console.log(`bundled git in ${git}`);
