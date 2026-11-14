// UI tests for the real furnish GUI, driven over WebDriver (tauri-driver on localhost:4444).
// Started by tests/ui/run.sh, which prepares the app's data dir and a fake package manager.
// Plain fetch, no test framework: each step names what it checks, the first failure stops the run.
import { readFileSync, existsSync, writeFileSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

const DRIVER = 'http://127.0.0.1:4444';
const [workDir, screenshot, container] = process.argv.slice(2);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
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
// Scrolls the target to the middle first: notifications sit bottom-right and WebKitWebDriver
// doesn't report a click that lands on one. Retries until the click goes through.
const click = async (xpath) =>
  waitFor(`click on ${xpath}`, async () => {
    const el = await find(xpath);
    await wd('POST', s('/execute/sync'), {
      script: "arguments[0].scrollIntoView({ block: 'center', inline: 'center' });",
      args: [{ [ELEMENT]: el }],
    });
    await wd('POST', s(`/element/${el}/click`), {});
    return true;
  });
const text = async (xpath) => wd('GET', s(`/element/${await find(xpath)}/text`));
const typeInto = async (xpath, value) => wd('POST', s(`/element/${await find(xpath)}/value`), { text: value });
// WebKitWebDriver reports content of a closed <details> as displayed, so check `open` instead.
const prop = async (xpath, name) => wd('GET', s(`/element/${await find(xpath)}/property/${name}`));
const isOpen = async (xpath) => wd('GET', s(`/element/${await find(xpath)}/property/open`));
const button = (label) => `//button[normalize-space()='${label}']`;
const card = (app) => `//article[.//h3[normalize-space()='${app}']]`;
const log = (file) => (existsSync(`${workDir}/${file}`) ? readFileSync(`${workDir}/${file}`, 'utf8') : '');

const steps = [
  ['the organization branding: name, logo and colors', async () => {
    // The logo ships with the preset collection, which the app clones on launch.
    await waitFor('logo from the collection', async () => (await prop("//nav/button[contains(@class,'brand')]//img", 'src')).startsWith('data:image/svg'), 30000);
    if ((await text("//nav/button[contains(@class,'brand')]")) !== 'Acme Apps') throw new Error('name not shown');
    const title = await wd('GET', s('/title'));
    if (title !== 'Acme Apps') throw new Error(`window title: ${title}`);
    const accent = await wd('POST', s('/execute/sync'), {
      script: "return getComputedStyle(document.documentElement).getPropertyValue('--accent').trim()", args: [],
    });
    if (accent !== '#e30613') throw new Error(`accent: ${accent}`);
  }],
  ['Discover recommends apps for the role; cards show the collection avatar', async () => {
    // The policy answers the fixture collection's role question: developer.
    const row = "//div[contains(@class,'grid') and contains(@class,'recommended')]";
    // By the cards' headings in the DOM (WebKit's element text leaves them out).
    const inRow = (app) => findAll(`${row}//h3[normalize-space()='${app}']`).then((l) => l.length > 0);
    await waitFor('hyperfine and devtool recommended', async () => (await inRow('hyperfine')) && (await inRow('devtool')), 30000);
    if (await inRow('toolbox')) throw new Error('toolbox recommended');
    if (!(await findAll("//article//span[contains(@class,'collection')]//img")).length) throw new Error('no collection avatar on cards');
  }],
  ['catalog lists hyperfine as installable', async () => {
    await waitFor('hyperfine card with Install', () => find(`${card('hyperfine')}${button('Install')}`));
  }],
  ['collections update on launch: an app added upstream appears', async () => {
    await waitFor('fresh app from the update', () => find(card('fresh')), 30000);
  }],
  ['apps without a resource for this OS are hidden', async () => {
    if ((await findAll(card('nowhere'))).length) throw new Error('nowhere is shown');
  }],
  ['a missing tool shows what is needed; prerequisites install first', async () => {
    await waitFor('brew-only shows what it needs', async () => (await text(card('brew-only'))).includes('Needs Homebrew'));
    if ((await findAll(`${card('brew-only')}${button('Install')}`)).length) throw new Error('blocked app offers Install');
    await click(`${card('brewed')}${button('Install')}`); // brewed requires the homebrew app
    await waitFor('homebrew installed', () => find(`${card('homebrew')}//*[normalize-space()='Installed']`), 30000);
    await waitFor('brewed installed', () => find(`${card('brewed')}//*[normalize-space()='Installed']`), 30000);
    await waitFor('brew-only installable now', () => find(`${card('brew-only')}${button('Install')}`));
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
  ['refusing the password dialog cancels quietly instead of failing', async () => {
    await click(button('Discover'));
    writeFileSync(`${workDir}/deny`, '');
    try {
      await click(`${card('toolbox')}${button('Install')}`);
      await waitFor('cancelled notice', () => find("//div[contains(@class,'toast') and contains(., 'Installation of toolbox cancelled')]"), 30000);
      if ((await findAll("//div[contains(@class,'toast') and contains(@class,'error')]")).length) throw new Error('shown as an error');
      await waitFor('still installable', () => find(`${card('toolbox')}${button('Install')}`));
    } finally {
      rmSync(`${workDir}/deny`);
    }
  }],
  ['abort lets the current step finish and skips the rest', async () => {
    const panel = "//details[summary[contains(., 'Steps')]]";
    await click(`${card('toolbox')}//button[@aria-label='Open toolbox']`);
    if (!(await isOpen(panel))) await click(`${panel}/summary`);
    await click("//button[contains(@class,'action') and normalize-space()='Install']");
    await waitFor('first step running', () => find(`${panel}//li[contains(@class,'running') and contains(., 'toolbox-a')]`));
    await click("//button[@aria-label='Abort']");
    await waitFor('stopping notice', async () => (await text(panel)).includes('Stopping after this step'));
    await waitFor('run cancelled', async () => (await text(panel)).includes('Installation of toolbox cancelled'), 30000);
    if (!(await text(`${panel}//li[contains(., 'toolbox-a')]`)).includes('Installed')) throw new Error('first step did not finish');
    if (!(await text(`${panel}//li[contains(., 'toolbox-b')]`)).includes('Not run')) throw new Error('second step ran');
  }],
  ['force stop interrupts the running step', async () => {
    const panel = "//details[summary[contains(., 'Steps')]]";
    await click("//button[contains(@class,'action') and normalize-space()='Install']");
    await waitFor('second step running', () => find(`${panel}//li[contains(@class,'running') and contains(., 'toolbox-b')]`));
    await click("//button[@aria-label='Abort']");
    await click("//button[@aria-label='Force stop']");
    await waitFor('run cancelled', async () => (await text(panel)).includes('Installation of toolbox cancelled'), 30000);
    if (!(await findAll(`${panel}//li[contains(@class,'cancelled') and contains(., 'toolbox-b')]`)).length) throw new Error('step not marked cancelled');
    if (existsSync(`${workDir}/pkgs/toolbox-b`)) throw new Error('toolbox-b got installed anyway');
  }],
  ['installs clicked in quick succession queue up; queued items can be removed', async () => {
    await click(button('Discover'));
    for (const app of ['toolbox', 'extras', 'fresh']) await click(`${card(app)}${button('Install')}`);
    await waitFor('two queued', async () => (await findAll("//article//*[contains(@aria-label, 'is queued')]")).length === 2);
    await click("//nav//button[starts-with(normalize-space(), 'Activity')]"); // label carries a count
    // The queue drains as it runs, so check for the last item rather than a count.
    const freshQueued = "//ol[contains(@class,'queue')]/li[.//*[normalize-space()='fresh']]";
    await waitFor('fresh listed in the queue', () => find(freshQueued));
    await click("//button[@aria-label='Remove fresh from the queue']");
    await waitFor('fresh gone from the queue', async () => !(await findAll(freshQueued)).length);
    await click(button('Discover'));
    for (const app of ['toolbox', 'extras']) {
      await waitFor(`${app} installed`, () => find(`${card(app)}//*[normalize-space()='Installed']`), 30000);
    }
    await waitFor('fresh still installable', () => find(`${card('fresh')}${button('Install')}`));
    if (existsSync(`${workDir}/pkgs/fresh`)) throw new Error('removed item was installed');
  }],
  ['kit uninstall: deselected items stay, the rest is removed', async () => {
    const item = (app) => `//ul[contains(@class,'items')]/li[.//*[contains(@class,'name') and normalize-space()='${app}']]`;
    await click(button('Discover'));
    await click("//button[@aria-label='Open kit starter']");
    await click(button('Uninstall apps…'));
    for (const app of ['hyperfine', 'toolbox', 'extras']) {
      if (!(await prop(`${item(app)}//input`, 'checked'))) throw new Error(`${app} not preselected`);
    }
    await click(`${item('extras')}//input`);
    await click(button('Uninstall 2 apps'));
    await click(button('Discover'));
    for (const app of ['hyperfine', 'toolbox']) {
      await waitFor(`${app} uninstalled`, () => find(`${card(app)}${button('Install')}`), 30000);
    }
    await waitFor('extras kept', () => find(`${card('extras')}//*[normalize-space()='Installed']`));
  }],
  ['kit install: required items are locked, deselected ones are skipped', async () => {
    const item = (app) => `//ul[contains(@class,'items')]/li[.//*[contains(@class,'name') and normalize-space()='${app}']]`;
    await click("//button[@aria-label='Open kit starter']");
    const required = `${item('hyperfine')}//input`;
    if (!(await prop(required, 'checked')) || !(await prop(required, 'disabled'))) throw new Error('required item not locked');
    if (!(await text(item('extras'))).includes('Installed')) throw new Error('installed item not marked');
    await click(`${item('toolbox')}//input`); // deselect the default-selected optional item
    await click(button('Install 1 app'));
    await click(button('Discover'));
    await waitFor('hyperfine installed', () => find(`${card('hyperfine')}//*[normalize-space()='Installed']`), 30000);
    await waitFor('toolbox skipped', () => find(`${card('toolbox')}${button('Install')}`));
  }],
  ['installing an app with optional dependencies lets users deselect them', async () => {
    const dep = (app) => `//dialog[@open]//li[.//*[contains(@class,'name') and normalize-space()='${app}']]`;
    await click(button('Discover'));
    await click(`${card('devtool')}${button('Install')}`);
    await waitFor('dependency dialog', () => find('//dialog[@open]'));
    if (!(await text(dep('hyperfine'))).includes('Installed')) throw new Error('installed dependency not marked');
    if (!(await prop(`${dep('toolbox')}//input`, 'checked'))) throw new Error('optional dependency not preselected');
    await click(`${dep('toolbox')}//input`);
    await click("//dialog[@open]//button[normalize-space()='Install']");
    await waitFor('devtool installed', () => find(`${card('devtool')}//*[normalize-space()='Installed']`), 30000);
    await waitFor('toolbox left out', () => find(`${card('toolbox')}${button('Install')}`));
  }],
  ['the detail page lists dependencies', async () => {
    await click(`${card('devtool')}//button[@aria-label='Open devtool']`);
    const t = await text("//section[contains(@class,'detail')]");
    for (const s of ['Dependencies', 'hyperfine', 'toolbox', 'extras', 'Optional']) {
      if (!t.includes(s)) throw new Error(`missing ${s}`);
    }
  }],
  ['updates install automatically, except for apps where that is turned off', async () => {
    writeFileSync(`${workDir}/pkgs/hyperfine.new`, '2.0\n');
    writeFileSync(`${workDir}/pkgs/extras.new`, '2.0\n');
    await click(button('Discover'));
    await click(`${card('extras')}//button[@aria-label='Open extras']`);
    await click("//label[contains(., 'Update automatically')]/input");
    await waitFor('opt-out saved', () => log('data/settings.json').includes('fixture/extras'));
    await click("//nav//button[starts-with(normalize-space(), 'Updates')]");
    await click(button('Check for updates'));
    await waitFor('hyperfine updated', () => log('pkgs/hyperfine').trim() === '2.0', 30000);
    await waitFor('extras still waiting', () => find(card('extras')));
    if (log('pkgs/extras').trim() === '2.0') throw new Error('extras was updated although turned off');
  }],
  ['automatic updates wait while the computer is busy, unless that is turned off', async () => {
    // Real CPU load from outside furnish: a busy loop on every core for 45 s.
    spawnSync('docker', ['exec', '-d', container, 'sh', '-c',
      'for i in $(seq $(nproc)); do timeout 45 sh -c "while :; do :; done" & done; wait']);
    await sleep(20000); // the 30 s test window fills with busy samples
    writeFileSync(`${workDir}/pkgs/hyperfine.new`, '3.0\n');
    await click("//nav//button[starts-with(normalize-space(), 'Updates')]");
    await click(button('Check for updates'));
    await waitFor('update postponed', async () => (await text("//*[contains(@class,'postponed')]")).includes('busy (CPU'), 20000);
    if (log('pkgs/hyperfine').trim() === '3.0') throw new Error('updated although the computer was busy');
    await click(button('Collections'));
    await click("//label[contains(., 'while this computer is busy')]/input");
    await waitFor('setting saved', () => log('data/settings.json').includes('"loadAwareUpdates": false'));
    await waitFor('hyperfine updated now', () => log('pkgs/hyperfine').trim() === '3.0', 30000);
  }],
  ['options change how an app installs; changing them later offers a reinstall', async () => {
    const select = "//select[@data-option='userInstall']";
    const choose = (label) => click(`${select}/option[normalize-space()='${label}']`);
    await click(button('Discover'));
    await click(`${card('cli-tool')}//button[@aria-label='Open cli-tool']`);
    if (!(await text("//section[contains(@class,'detail')]")).includes('Install just for me')) throw new Error('option label missing');
    await choose('Yes');
    await waitFor('app value saved', () => log('data/settings.json').includes('fixture/cli-tool'));
    await click("//button[contains(@class,'action') and normalize-space()='Install']");
    await waitFor('installed for the user', () => existsSync(`${workDir}/brew-pkgs/cli-tool`), 30000);
    if (existsSync(`${workDir}/pkgs/cli-tool`)) throw new Error('installed system-wide');
    await choose('No');
    await waitFor('app value changed', () => log('data/settings.json').includes('"userInstall": false'));
    await click(button('Reinstall with these options'));
    await waitFor('moved to a system-wide install', () => existsSync(`${workDir}/pkgs/cli-tool`) && !existsSync(`${workDir}/brew-pkgs/cli-tool`), 30000);
    await waitFor('banner gone', async () => !(await findAll(button('Reinstall with these options'))).length);
  }],
  ['collection options apply to all its apps', async () => {
    await click(button('Collections'));
    await click("//summary[contains(., 'Options for all fixture apps')]");
    await waitFor('collection option shown', async () => (await text("//details[contains(@class,'copts')]")).includes('Install just for me'));
  }],
  ['collections: add a repository, see its apps, remove it again', async () => {
    await click(button('Collections'));
    await typeInto("//input[@placeholder='git@github.com:your-org/apps.git']", `${workDir}/second`);
    await click(button('Add collection'));
    await waitFor('second collection listed', () => find("//li[.//*[normalize-space()='second']]"), 30000);
    // Its required question comes up right away, as a dropdown with labels.
    await waitFor('question dialog', () => find("//dialog[@open]//select[@data-question='role']"));
    if (!(await text('//dialog[@open]')).includes('What do you work on?')) throw new Error('question text missing');
    await click("//dialog[@open]//select[@data-question='role']/option[normalize-space()='Design']");
    await click("//dialog[@open]//button[normalize-space()='Save answers']");
    await waitFor('answer saved', () => /"second":\s*\{\s*"role":\s*"design"/.test(log('data/settings.json')));
    await click("//li[.//*[normalize-space()='second']]//button[normalize-space()='Remove']");
    await click("//button[contains(@class,'danger') and normalize-space()='Remove']");
    await waitFor('second collection gone', async () => !(await findAll("//li[.//*[normalize-space()='second']]")).length);
  }],
  ['automatic update settings are saved; locked ones are not editable', async () => {
    await click(button('Collections'));
    await click("//label[contains(., 'when Acme Apps starts')]/input");
    await waitFor('settings.json', () => log('data/settings.json').includes('"updateOnLaunch": false'));
    // The admin policy locks the interval.
    if (!(await prop('//select', 'disabled'))) throw new Error('locked interval is editable');
    if (!(await text("//label[contains(@class,'interval')]")).includes('Set by your organization')) throw new Error('no hint');
    if (log('data/settings.json').includes('updateIntervalMinutes')) throw new Error('locked setting saved for the user');
  }],
  ['collections the organization provides cannot be removed', async () => {
    const row = "//li[.//*[normalize-space()='fixture']]";
    if (!(await text(row)).includes('Managed by your organization')) throw new Error('not marked managed');
    if ((await findAll(`${row}//button[normalize-space()='Remove']`)).length) throw new Error('Remove offered');
  }],
  ['the logo leads back to Discover', async () => {
    await click("//button[@aria-label='Acme Apps, go to Discover']");
    await waitFor('Discover page', async () => (await text('//h1')) === 'Discover');
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
