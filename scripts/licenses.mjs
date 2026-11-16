// Writes THIRD-PARTY-LICENSES.html: the open-source components in furnish's installers and their
// license texts. The installers ship the file; CI regenerates it and fails if it changed.
//
//   node scripts/licenses.mjs        (needs cargo-about 0.9, git and gui/node_modules; clones DSC's
//                                     source into .cache/dsc/ and fetches the Git LFS license)
//
// It covers the Rust crates of furnish and furnish-gui (the workspace) and of the DSC resources
// built from source (packaging/dsc.json), for the targets in about.toml; the npm packages Vite
// bundles into the desktop app's interface; and the programs the installers contain.
import { execFileSync } from 'node:child_process';
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { dsc, dscSource } from './dsc-source.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const link = (url, text = url) => `<a href="${esc(url)}">${esc(text)}</a>`;
const pre = (text) => `<pre>${esc(text.trim())}</pre>`;
const slug = (s) => s.toLowerCase().replace(/[^a-z0-9]+/g, '-');

// Rust crates, from cargo-about. It fails on a license that about.toml doesn't accept; a crate
// whose license it can't determine fails here. Path crates are furnish's own code or, in DSC's
// workspace, DSC's (covered by the DSC section).
function cargoAbout(args, skip) {
  let out;
  try {
    out = execFileSync('cargo', ['about', 'generate', '--locked', '-c', `${root}about.toml`, '--format', 'json', ...args], {
      cwd: root,
      maxBuffer: 256 << 20,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
  } catch (e) {
    process.stderr.write(e.stderr ?? '');
    throw new Error(`cargo about ${args.join(' ')} failed`);
  }
  const about = JSON.parse(out);
  const licensed = new Set(about.licenses.flatMap((l) => l.used_by.map((u) => u.crate.id)));
  const unknown = about.crates.filter((c) => !skip(c.package) && !licensed.has(c.package.id));
  if (unknown.length) throw new Error(`no license found for ${unknown.map((c) => `${c.package.name} ${c.package.version}`).join(', ')}: clarify it in about.toml`);
  return about.licenses.map((l) => ({ ...l, used_by: l.used_by.filter((u) => !skip(u.crate)) })).filter((l) => l.used_by.length);
}
const dscSrc = dscSource();
const rustRuns = [
  cargoAbout(['--workspace'], (c) => c.name === 'furnish' || c.name === 'furnish-gui'),
  ...dsc.build.map(({ manifest, windowsOnly }) =>
    cargoAbout(['-m', `${dscSrc}/${dirname(manifest)}/Cargo.toml`, ...(windowsOnly ? ['--target', 'x86_64-pc-windows-msvc'] : [])], (c) => !c.source),
  ),
];
// license id -> license text -> crates using it
const rust = new Map();
for (const l of rustRuns.flat()) {
  const texts = rust.get(l.id) ?? rust.set(l.id, { name: l.name, texts: new Map() }).get(l.id);
  const crates = texts.texts.get(l.text) ?? texts.texts.set(l.text, new Map()).get(l.text);
  for (const { crate } of l.used_by) crates.set(`${crate.name} ${crate.version}`, crate);
}
const rustCount = new Set([...rust.values()].flatMap((l) => [...l.texts.values()].flatMap((c) => [...c.keys()]))).size;

// npm packages in the desktop app's interface: what Vite's build of gui/ actually bundles, plus
// Vite's and Rolldown's runtime helpers when the bundle contains them.
const modules = `${root}gui/node_modules/`;
const { build } = await import(`${modules}vite/dist/node/index.js`);
const bundled = new Set();
await build({
  root: `${root}gui`,
  configFile: `${root}gui/vite.config.js`,
  logLevel: 'silent',
  build: { write: false },
  plugins: [
    {
      name: 'licenses',
      generateBundle(_, bundle) {
        for (const chunk of Object.values(bundle))
          for (const id of chunk.moduleIds ?? []) {
            const name = id.match(/node_modules\/((?:@[^/]+\/)?[^/]+)\//)?.[1] ?? id.match(/^\0(vite|rolldown)\//)?.[1];
            if (name) bundled.add(name);
          }
      },
    },
  ],
});
const npmByLicense = new Map();
for (const name of [...bundled].sort()) {
  const dir = `${modules}${name}/`;
  const pkg = JSON.parse(readFileSync(`${dir}package.json`, 'utf8'));
  const files = readdirSync(dir).filter((f) => /^(licen[cs]e|copying)/i.test(f) && !f.endsWith('.spdx')).sort();
  if (!files.length) throw new Error(`${name} has no license file`);
  // Copyleft in the interface would need more than a notice; decide before shipping it.
  if (/GPL/i.test(pkg.license)) throw new Error(`${name} ${pkg.version} is ${pkg.license}`);
  const repo = (pkg.repository?.url ?? pkg.repository ?? '').replace(/^git\+/, '').replace(/\.git$/, '');
  const list = npmByLicense.get(pkg.license) ?? npmByLicense.set(pkg.license, []).get(pkg.license);
  list.push(`<p>${repo ? link(repo, `${name} ${pkg.version}`) : esc(`${name} ${pkg.version}`)}</p>${files.map((f) => pre(readFileSync(dir + f, 'utf8'))).join('')}`);
}

// Programs the Windows installer contains (see gui/scripts/bundle-resources.mjs).
const git = JSON.parse(readFileSync(`${root}gui/scripts/bundled-git.json`, 'utf8'));
const lfsLicense = await (await fetch(git.lfs.source.replace('github.com', 'raw.githubusercontent.com').replace('/tree/', '/') + '/LICENSE.md')).text();
const dscLicense = readFileSync(`${dscSrc}/LICENSE`, 'utf8');

// In about.toml's order of preference, which decides the license listed for a crate offering a choice.
const accepted = [...readFileSync(`${root}about.toml`, 'utf8').match(/^accepted = \[([^\]]*)\]/m)[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
const rustIds = [...rust.keys()].sort((a, b) => accepted.indexOf(a) - accepted.indexOf(b));
const crateCount = (id) => new Set([...rust.get(id).texts.values()].flatMap((c) => [...c.keys()])).size;
const rustSections = rustIds.map((id) => {
  const { name, texts } = rust.get(id);
  const blocks = [...texts]
    .map(([text, crates]) => {
      const names = [...crates.values()]
        .sort((a, b) => a.name.localeCompare(b.name))
        .map((c) => link(c.repository || `https://crates.io/crates/${c.name}`, `${c.name} ${c.version}`))
        .join(', ');
      return { names, html: `<p>${names}</p>${pre(text)}` };
    })
    .sort((a, b) => a.names.localeCompare(b.names));
  return `<section id="rust-${slug(id)}"><h3>${esc(name)} (${esc(id)})</h3>\n${blocks.map((b) => b.html).join('\n')}</section>`;
});
const npmIds = [...npmByLicense.keys()].sort();

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
<ul>
<li><a href="#programs">Bundled programs</a>: DSC, and MinGit and Git LFS in the Windows installer</li>
<li><a href="#npm">The desktop app's interface</a>: ${npmIds.map((id) => `${esc(id)} (${npmByLicense.get(id).length})`).join(', ')}</li>
<li><a href="#rust">Rust crates</a> (${rustCount}): ${rustIds.map((id) => `<a href="#rust-${slug(id)}">${esc(id)}</a> (${crateCount(id)})`).join(', ')}</li>
</ul>

<h2 id="programs">Bundled programs</h2>
<section>
<h3>${link(`${dsc.source}/releases/tag/v${dsc.version}`, `DSC ${dsc.version} resources`)} (MIT), in every installer</h3>
<p>DSC's own resources and the <code>dsc</code> program, from Microsoft's release, and the resources ${dsc.build.map((b) => `<code>${esc(b.crate)}</code>`).join(' and ')} built from its source code: ${link(`${dsc.source}/tree/v${dsc.version}`)}. The license notices of the release's components are in <code>dsc/NOTICE.txt</code> in furnish's installation folder (Linux: <code>/usr/lib/furnish/dsc</code>, macOS: <code>furnish.app/Contents/Resources/dsc</code>); the Rust crates of the resources built from source are listed under <a href="#rust">Rust crates</a>.</p>
${pre(dscLicense)}
</section>
<section>
<h3>${link('https://github.com/git-for-windows/git/wiki/MinGit', `MinGit ${git.mingit.version}`)} (Git for Windows), in the Windows installer</h3>
<p>git is licensed under the GNU General Public License, version 2; the other programs and libraries in MinGit under their own licenses. Their license texts are in the folder <code>git</code> in furnish's installation folder: <code>LICENSE.txt</code> and the folders <code>ucrt64\\share\\licenses</code>, <code>ucrt64\\doc</code> and <code>usr\\share\\licenses</code>.</p>
<p>The source code of this version: ${link(git.mingit.source)}, and of its components: ${link('https://github.com/git-for-windows')}.</p>
</section>
<section>
<h3>${link('https://git-lfs.com', `Git LFS ${git.lfs.version}`)} (MIT), in the Windows installer</h3>
<p>Source code: ${link(git.lfs.source)}</p>
${pre(lfsLicense)}
</section>

<h2 id="npm">The desktop app's interface</h2>
<p>The JavaScript packages and the font compiled into the desktop app.</p>
${npmIds.map((id) => `<section><h3>${esc(id)}</h3>\n${npmByLicense.get(id).join('\n')}</section>`).join('\n')}

<h2 id="rust">Rust crates</h2>
<p>Compiled into furnish, its desktop app and DSC's resources built from source, for Windows, macOS and Linux. Crates offering a choice of licenses are listed under the first one in this order: ${esc(accepted.join(', '))}.</p>
${rustSections.join('\n')}
</body>
</html>
`;
writeFileSync(`${root}THIRD-PARTY-LICENSES.html`, html);
console.log(`THIRD-PARTY-LICENSES.html: ${rustCount} crates, ${bundled.size} npm packages, 3 bundled programs`);
