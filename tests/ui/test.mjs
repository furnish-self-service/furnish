// UI tests for the real furnish GUI, driven over WebDriver (tauri-driver on localhost:4444).
// Started by tests/ui/run.sh, which prepares the app's data dir and a fake package manager.
// Plain fetch, no test framework: each step names what it checks, the first failure stops the run.
import { readFileSync, existsSync, writeFileSync } from 'node:fs';

const DRIVER = 'http://127.0.0.1:4444';
const [workDir, screenshot] = process.argv.slice(2);
let session;

async function wd(method, path, body) {
  const res = await fetch(`${DRIVER}${path}`, {
    method,
    headers: { 'content-type': 'application/json' },
    body: body && JSON.stringify(body),
  });
  const json = await res.json();
  if (!res.ok) throw new Error(`${method} ${path}: ${JSON.stringify(json.value)}`);
  return json.value;
}
const s = (path) => `/session/${session}${path}`;
const ELEMENT = 'element-6066-11e4-a52e-4f735466cecf';

/** Polls until `fn` returns a truthy value (or throws the last error). */
async function waitFor(what, fn, timeout = 20000) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try {
      const v = await fn();
      if (v) return v;
    } catch (e) {
      last = e;
    }
    await new Promise((r) => setTimeout(r, 250));
  }
  throw new Error(`timed out waiting for ${what}${last ? `: ${last.message}` : ''}`);
}
const find = async (xpath) => (await wd('POST', s('/element'), { using: 'xpath', value: xpath }))[ELEMENT];
const findAll = async (xpath) => (await wd('POST', s('/elements'), { using: 'xpath', value: xpath })).map((e) => e[ELEMENT]);
const click = async (xpath) => wd('POST', s(`/element/${await waitFor(xpath, () => find(xpath))}/click`), {});
const text = async (xpath) => wd('GET', s(`/element/${await find(xpath)}/text`));
const typeInto = async (xpath, value) => wd('POST', s(`/element/${await find(xpath)}/value`), { text: value });
// WebKitWebDriver reports content of a closed <details> as displayed, so check `open` instead.
const isOpen = async (xpath) => wd('GET', s(`/element/${await find(xpath)}/property/open`));
const button = (label) => `//button[normalize-space()='${label}']`;
const card = (app) => `//article[.//h3[normalize-space()='${app}']]`;
const log = (file) => (existsSync(`${workDir}/${file}`) ? readFileSync(`${workDir}/${file}`, 'utf8') : '');

const steps = [
  ['catalog lists hyperfine as installable', async () => {
    await waitFor('hyperfine card with Install', () => find(`${card('hyperfine')}${button('Install')}`));
  }],
  ['apps without a resource for this OS are hidden', async () => {
    if ((await findAll(card('nowhere'))).length) throw new Error('nowhere is shown');
  }],
  ['install from the card finishes and shows Installed', async () => {
    await click(`${card('hyperfine')}${button('Install')}`);
    await waitFor('Installed state', () => find(`${card('hyperfine')}//*[normalize-space()='Installed']`), 30000);
  }],
  ['elevation used the graphical prompt (pkexec), never sudo in a terminal', async () => {
    if (log('sudo.log')) throw new Error(`sudo was called: ${log('sudo.log')}`);
    if (!log('pkexec.log').includes('pacman -S')) throw new Error(`pkexec not used: '${log('pkexec.log')}'`);
  }],
  ['detail page hides the DSC configuration until revealed', async () => {
    await click(`${card('hyperfine')}//button[@aria-label='Open hyperfine']`);
    const panel = "//details[summary[contains(., 'DSC configuration')]]";
    if (await isOpen(panel)) throw new Error('configuration open before click');
    if (await isOpen("//details[summary[contains(., 'Activity log')]]")) throw new Error('log open before click');
    await click(`${panel}/summary`);
    await waitFor('configuration shown', async () => (await isOpen(panel)) && (await text(`${panel}//pre`)).includes('Furnish/Package'));
  }],
  ['preview lists the concrete steps and changes nothing', async () => {
    const panel = "//details[summary[contains(., 'Steps')]]";
    if (await isOpen(panel)) throw new Error('preview open before click');
    await click(`${panel}/summary`);
    // Installed, so the preview shows uninstalling: the exact command and who runs it.
    await waitFor('uninstall command in preview', async () => {
      const t = await text(panel);
      return t.includes('pacman -R --noconfirm hyperfine') && /runs as administrator/i.test(t);
    });
    if (log('pkexec.log').includes('-R')) throw new Error('the preview ran the removal');
    await click(`${panel}//button[normalize-space()='Install']`);
    await waitFor('install preview: nothing to do', async () => (await text(panel)).includes('change nothing'));
  }],
  ['activity log shows what the install did, after one click', async () => {
    const lines = "//details[summary[contains(., 'Activity log')]]//ol";
    await click("//summary[contains(., 'Activity log')]");
    await waitFor('log mentions pacman', async () => (await text(lines)).includes('pacman -S'));
  }],
  ['uninstall needs confirmation and returns the app to not installed', async () => {
    await click(button('Uninstall'));
    await click("//button[contains(@class,'danger') and normalize-space()='Uninstall']");
    await waitFor('Not installed', () => find("//*[normalize-space()='Not installed']"), 30000);
  }],
  ['steps panel follows an install live: current step, then results with durations', async () => {
    const panel = "//details[summary[contains(., 'Steps')]]";
    if (!(await isOpen(panel))) await click(`${panel}/summary`);
    await click("//button[contains(@class,'action') and normalize-space()='Install']");
    await waitFor('a running step', () => find(`${panel}//li[contains(@class,'running')]`));
    if (!/step \d of \d/.test(await text(`${panel}//p[contains(@class,'summary')]`))) throw new Error('no step counter');
    await waitFor('run finished', async () => /Installed hyperfine in/.test(await text(`${panel}//p[contains(@class,'summary')]`)), 30000);
    const done = await text(`${panel}//li[contains(@class,'done')]`);
    if (!/Installed/.test(done) || !/\d+(\.\d)? (ms|s)/.test(done)) throw new Error(`done step lacks result/duration: ${done}`);
    if (!(await findAll(`${panel}//li[contains(@class,'skipped')]`)).length) throw new Error('skipped step missing');
  }],
  ['collections: add a repository, see its apps, remove it again', async () => {
    await click(button('Collections'));
    await typeInto("//input[@placeholder='git@github.com:your-org/apps.git']", `${workDir}/second`);
    await click(button('Add collection'));
    await waitFor('second collection listed', () => find("//li[.//*[normalize-space()='second']]"), 30000);
    await click("//li[.//*[normalize-space()='second']]//button[normalize-space()='Remove']");
    await click("//button[contains(@class,'danger') and normalize-space()='Remove']");
    await waitFor('second collection gone', async () => !(await findAll("//li[.//*[normalize-space()='second']]")).length);
  }],
];

let failed = false;
try {
  session = (
    await waitFor('tauri-driver', () =>
      wd('POST', '/session', {
        capabilities: { alwaysMatch: { 'tauri:options': { application: '/usr/local/bin/furnish-gui' } } },
      }),
    )
  ).sessionId;
  for (const [name, step] of steps) {
    try {
      await step();
      console.log(`ok    ${name}`);
    } catch (e) {
      console.log(`FAIL  ${name}\n      ${e.message}`);
      failed = true;
      break;
    }
  }
} finally {
  if (session) {
    try {
      writeFileSync(screenshot, Buffer.from(await wd('GET', s('/screenshot')), 'base64'));
      console.log(`screenshot: ${screenshot}`);
    } catch {}
    await wd('DELETE', s('')).catch(() => {});
  }
}
process.exit(failed ? 1 : 0);
