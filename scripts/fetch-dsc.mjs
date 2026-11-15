// Put DSC's own resources into a folder, where furnish finds them (engine::bundled_dsc_dir): the
// release of the DSC version furnish embeds, without its samples and test tools, and the
// resources the release lacks, built from the same source. Everything pinned in packaging/dsc.json.
//
//   node scripts/fetch-dsc.mjs <folder> [target triple]
//
// The target defaults to FURNISH_CLI_TARGET, else rustc's host. Downloads and builds are cached
// in .cache/dsc/. Needs git and cargo; tar that reads .zip for Windows (Windows has one).
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { basename, dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const dsc = JSON.parse(readFileSync(`${root}packaging/dsc.json`, 'utf8'));
const dest = resolve(process.argv[2] ?? '');
if (!process.argv[2]) throw new Error('usage: node scripts/fetch-dsc.mjs <folder> [target triple]');
const target =
  process.argv[3] || process.env.FURNISH_CLI_TARGET || execFileSync('rustc', ['-vV']).toString().match(/host: (\S+)/)[1];
const windows = target.includes('-windows-');
const key = windows || target.includes('-apple-') ? target.replace('-gnu', '') : `${target.split('-')[0]}-linux`;
const release = dsc.releases[key];
if (!release) throw new Error(`no DSC release for ${target} in packaging/dsc.json`);
const cache = `${root}.cache/dsc/`;
mkdirSync(cache, { recursive: true });
const run = (cmd, args, opts = {}) => execFileSync(cmd, args, { stdio: ['ignore', 'inherit', 'inherit'], ...opts });

// The release, checked by SHA-256.
const archive = cache + basename(release.url);
if (!existsSync(archive)) {
  const res = await fetch(release.url);
  if (!res.ok) throw new Error(`${release.url}: ${res.status}`);
  writeFileSync(`${archive}.part`, Buffer.from(await res.arrayBuffer()));
  renameSync(`${archive}.part`, archive);
}
const actual = createHash('sha256').update(readFileSync(archive)).digest('hex');
if (actual !== release.sha256) {
  rmSync(archive);
  throw new Error(`${release.url}: SHA-256 ${actual}, expected ${release.sha256}`);
}
const unpacked = `${cache}${key}/`;
rmSync(unpacked, { recursive: true, force: true });
mkdirSync(unpacked);
// Windows' own tar (bsdtar) extracts zip files; Git Bash's GNU tar on PATH doesn't.
const tar = process.platform === 'win32' ? `${process.env.SystemRoot}\\System32\\tar.exe` : archive.endsWith('.zip') ? 'bsdtar' : 'tar';
run(tar, ['-xf', archive, '-C', unpacked]);
rmSync(dest, { recursive: true, force: true });
const excluded = (file) => dsc.exclude.includes(basename(file).replace(/\.exe$/i, ''));
cpSync(unpacked, dest, { recursive: true, filter: (file) => !excluded(file) });

// Resources the release doesn't contain, from the same commit, without DSC's .cargo/config.toml: it
// points at Microsoft's internal crate mirror.
const src = `${cache}src-${dsc.commit}`;
if (!existsSync(src)) {
  run('git', ['-c', 'advice.detachedHead=false', 'clone', '-q', '--depth', '1', '--branch', `v${dsc.version}`, dsc.source, `${src}.part`]);
  rmSync(`${src}.part/.cargo`, { recursive: true });
  renameSync(`${src}.part`, src);
}
const head = execFileSync('git', ['-C', src, 'rev-parse', 'HEAD']).toString().trim();
if (head !== dsc.commit) throw new Error(`DSC v${dsc.version} is ${head}, expected ${dsc.commit}`);
const exe = windows ? '.exe' : '';
const installed = `${cache}build-${target}/`;
// Like DSC's own Windows builds: no dependency on the optional Visual C++ runtime.
const env = { ...process.env };
if (target.endsWith('-msvc')) env[`CARGO_TARGET_${target.toUpperCase().replace(/-/g, '_')}_RUSTFLAGS`] = '-Ctarget-feature=+crt-static';
for (const { crate, manifest, windowsOnly } of dsc.build) {
  if (windowsOnly && !windows) continue;
  run('cargo', ['install', '-q', '--locked', '--path', `${src}/${dirname(manifest)}`, '--root', installed, '--target-dir', `${cache}target`, '--target', target], { env });
  cpSync(`${installed}bin/${crate}${exe}`, `${dest}/${crate}${exe}`);
  cpSync(`${src}/${manifest}`, `${dest}/${basename(manifest)}`);
}
console.log(`DSC ${dsc.version} resources for ${target} in ${dest}`);
