// DSC's source code at the commit in packaging/dsc.json, cloned once into .cache/dsc/, without
// DSC's .cargo/config.toml: it points at Microsoft's internal crate mirror.
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
export const dsc = JSON.parse(readFileSync(`${root}packaging/dsc.json`, 'utf8'));

export function dscSource() {
  const cache = `${root}.cache/dsc/`;
  const src = `${cache}src-${dsc.commit}`;
  if (!existsSync(src)) {
    mkdirSync(cache, { recursive: true });
    rmSync(`${src}.part`, { recursive: true, force: true });
    execFileSync('git', ['-c', 'advice.detachedHead=false', 'clone', '-q', '--depth', '1', '--branch', `v${dsc.version}`, dsc.source, `${src}.part`], { stdio: 'inherit' });
    rmSync(`${src}.part/.cargo`, { recursive: true });
    renameSync(`${src}.part`, src);
  }
  const head = execFileSync('git', ['-C', src, 'rev-parse', 'HEAD']).toString().trim();
  if (head !== dsc.commit) throw new Error(`DSC v${dsc.version} is ${head}, expected ${dsc.commit}`);
  return src;
}
