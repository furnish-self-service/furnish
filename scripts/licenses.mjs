// Writes THIRD-PARTY-LICENSES.html: the open-source components in furnish's installers and their
// license texts. The installers ship the file; CI regenerates it and fails if it changed.
//
//   node scripts/licenses.mjs        (needs cargo-about 0.9 and gui/node_modules; fetches the
//                                     Git LFS license from GitHub)
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const link = (url, text = url) => `<a href="${esc(url)}">${esc(text)}</a>`;
const pre = (text) => `<pre>${esc(text.trim())}</pre>`;

// Rust crates: cargo-about groups crates that share a license text (see about.toml).
const about = JSON.parse(
  execFileSync('cargo', ['about', 'generate', '--format', 'json'], { cwd: root, maxBuffer: 64 << 20, stdio: ['ignore', 'pipe', 'ignore'] }),
);
const own = new Set(['furnish', 'furnish-gui']);
const crates = about.licenses
  .map((l) => ({ ...l, used_by: l.used_by.filter((u) => !own.has(u.crate.name)) }))
  .filter((l) => l.used_by.length)
  .map((l) => {
    const names = l.used_by
      .map((u) => link(u.crate.repository || `https://crates.io/crates/${u.crate.name}`, `${u.crate.name} ${u.crate.version}`))
      .join(', ');
    return `<section><h3>${esc(l.name)}</h3><p>Used by ${names}</p>${pre(l.text)}</section>`;
  });

// The desktop app's interface: what its build compiles in (Svelte's runtime, Tauri's API, the font).
const modules = `${root}gui/node_modules/`;
const npm = ['svelte', '@tauri-apps/api', '@fontsource-variable/bricolage-grotesque'].map((name) => {
  const pkg = JSON.parse(readFileSync(`${modules}${name}/package.json`, 'utf8'));
  const files = readdirSync(`${modules}${name}`).filter((f) => /^licen[cs]e/i.test(f) && !f.endsWith('.spdx')).sort();
  const texts = files.map((f) => pre(readFileSync(`${modules}${name}/${f}`, 'utf8'))).join('');
  const repo = (pkg.repository?.url ?? pkg.repository ?? '').replace(/^git\+/, '').replace(/\.git$/, '');
  return `<section><h3>${repo ? link(repo, `${name} ${pkg.version}`) : esc(`${name} ${pkg.version}`)} (${esc(pkg.license)})</h3>${texts}</section>`;
});

// Programs the Windows installer contains (see gui/scripts/bundle-resources.mjs).
const git = JSON.parse(readFileSync(`${root}gui/scripts/bundled-git.json`, 'utf8'));
const dsc = JSON.parse(readFileSync(`${root}packaging/dsc.json`, 'utf8'));
const lfsLicense = await (await fetch(git.lfs.source.replace('github.com', 'raw.githubusercontent.com').replace('/tree/', '/') + '/LICENSE.md')).text();

const html = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>furnish: open-source licenses</title>
<style>
  :root { color-scheme: light dark; }
  body { max-width: 52rem; margin: 2rem auto; padding: 0 1rem; font: 15px/1.5 system-ui, sans-serif; }
  pre { max-height: 16rem; overflow: auto; padding: 0.75rem; border: 1px solid #8884; border-radius: 6px; white-space: pre-wrap; font-size: 12px; }
  section { margin-bottom: 1.5rem; }
  h3 { margin-bottom: 0.25rem; }
</style>
</head>
<body>
<h1>Open-source licenses</h1>
<p>furnish is open source under the ${link('https://github.com/furnish-self-service/furnish/blob/main/LICENSE', 'MIT license')}. It contains the components below, under their own licenses.</p>

<h2>In every installer</h2>
<section>
<h3>${link(`${dsc.source}/releases/tag/v${dsc.version}`, `DSC ${dsc.version} resources`)} (MIT)</h3>
<p>DSC's own resources and the <code>dsc</code> program, from Microsoft's release and built from its source code: ${link(`${dsc.source}/tree/v${dsc.version}`)}. The license notices of their components are in <code>dsc/NOTICE.txt</code> in furnish's installation folder (Linux: <code>/usr/lib/furnish/dsc</code>, macOS: <code>furnish.app/Contents/Resources/dsc</code>).</p>
</section>

<h2>In the Windows installer</h2>
<section>
<h3>${link('https://github.com/git-for-windows/git/wiki/MinGit', `MinGit ${git.mingit.version}`)} (Git for Windows)</h3>
<p>git is licensed under the GNU General Public License, version 2; the other programs and libraries in MinGit under their own licenses. Their license texts are in the folder <code>git</code> in furnish's installation folder: <code>LICENSE.txt</code> and the folders <code>ucrt64\\share\\licenses</code>, <code>ucrt64\\doc</code> and <code>usr\\share\\licenses</code>.</p>
<p>The source code of this version: ${link(git.mingit.source)}, and of its components: ${link('https://github.com/git-for-windows')}.</p>
</section>
<section>
<h3>${link('https://git-lfs.com', `Git LFS ${git.lfs.version}`)} (MIT)</h3>
<p>Source code: ${link(git.lfs.source)}</p>
${pre(lfsLicense)}
</section>

<h2>In the desktop app's interface</h2>
${npm.join('\n')}

<h2>Rust crates</h2>
${crates.join('\n')}
</body>
</html>
`;
writeFileSync(`${root}THIRD-PARTY-LICENSES.html`, html);
console.log(`THIRD-PARTY-LICENSES.html: ${about.crates.length} crates, ${npm.length} npm packages, 3 bundled programs`);
