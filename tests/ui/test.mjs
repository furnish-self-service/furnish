// UI tests for the real furnish GUI, driven over WebDriver (tauri-driver on localhost, port DRIVER_PORT).
// Started by tests/ui/run.sh, which prepares the app's data dir and a fake package manager.
// Plain fetch, no test framework: each step names what it checks, the first failure stops the run.
import { readFileSync, existsSync, writeFileSync, rmSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

// run.sh picks the port, so runs side by side (host network) don't meet each other's driver.
const DRIVER = `http://127.0.0.1:${process.env.DRIVER_PORT || 4444}`;
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
// A sidebar entry; Collections may carry a count after its name.
const nav = (label) => `//nav//button[starts-with(normalize-space(), '${label}')]`;
const card = (app) => `//article[.//h3[normalize-space()='${app}']]`;
// Extra screenshots of states worth seeing, next to the final one.
const snap = async (name) =>
  writeFileSync(screenshot.replace(/\.png$/, `-${name}.png`), Buffer.from(await wd('GET', s('/screenshot')), 'base64'));
const log = (file) => (existsSync(`${workDir}/${file}`) ? readFileSync(`${workDir}/${file}`, 'utf8') : '');
// The sign-in entry furnish writes (HOME=/tmp in the container).
const AUTOSTART = '/tmp/.config/autostart/furnish.desktop';
const autostartEntry = () => spawnSync('docker', ['exec', container, 'cat', AUTOSTART], { encoding: 'utf8' }).stdout;

/** Start furnish (again: a new WebDriver session is a new process). */
async function startSession(application = '/usr/local/bin/furnish-gui') {
  if (session) await wd('DELETE', s('')).catch(() => {});
  session = null;
  session = (
    await waitFor('tauri-driver', () =>
      wd('POST', '/session', {
        capabilities: { alwaysMatch: { 'tauri:options': { application } } },
      }),
    )
  ).sessionId;
}

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
  ['Steps previews the NixOS resource in the desktop app', async () => {
    await click(`${card('nixos-preview')}//button[@aria-label='Open nixos-preview']`);
    const panel = "//details[summary[contains(., 'Steps')]]";
    await click(`${panel}/summary`);
    await waitFor('NixOS removal preview', async () => (await text(panel)).includes('change nothing'));
    await click(`${panel}//button[normalize-space()='Install']`);
    await waitFor('NixOS install preview', async () => (await text(panel)).includes('Installing nixos-preview'));
    await click(nav('Discover'));
  }],
  ['Discover recommends apps for the role; cards show the collection avatar', async () => {
    // The policy answers the fixture collection's role question: developer.
    const row = "//div[contains(@class,'grid') and contains(@class,'recommended')]";
    // By the cards' headings in the DOM (WebKit's element text leaves them out).
    const inRow = (app) => findAll(`${row}//h3[normalize-space()='${app}']`).then((l) => l.length > 0);
    await waitFor('hyperfine and devtool recommended', async () => (await inRow('hyperfine')) && (await inRow('devtool')), 30000);
    if (await inRow('toolbox')) throw new Error('toolbox recommended');
    // Avatars come with the collection list, which loads separately (and again after the update on launch).
    await waitFor('collection avatar on cards', async () => (await findAll("//article//span[contains(@class,'collection')]//img")).length > 0);
    // The fixture's iconDark: offered to dark mode beside the avatar, as an image of its own.
    const dark = "//article//span[contains(@class,'collection')]//picture/source[@media='(prefers-color-scheme: dark)'][starts-with(@srcset,'data:image/svg+xml;base64,')]";
    await waitFor('dark-mode avatar on the cards', async () => (await findAll(dark)).length > 0);
  }],
  ['catalog lists hyperfine as installable', async () => {
    await waitFor('hyperfine card with Install', () => find(`${card('hyperfine')}${button('Install')}`));
  }],
  ['collections update on launch: an app added upstream appears', async () => {
    await waitFor('fresh app from the update', () => find(card('fresh')), 30000);
  }],
  ['apps without a resource for this OS are hidden', async () => {
    // Cards show until the app's status arrives; "not available here" then hides it.
    await waitFor('nowhere hidden', async () => !(await findAll(card('nowhere'))).length);
  }],
  ['a missing tool shows what is needed; prerequisites install first', async () => {
    await waitFor('brew-only shows what it needs', async () => (await text(card('brew-only'))).includes('Needs Homebrew'));
    if ((await findAll(`${card('brew-only')}${button('Install')}`)).length) throw new Error('blocked app offers Install');
    writeFileSync(`${workDir}/deny`, '');
    try {
      await click(`${card('brewed')}${button('Install')}`);
      await click(nav('Activity'));
      await waitFor('brewed is waiting for its cancelled prerequisite', () => find(button('Retry homebrew')));
    } finally {
      rmSync(`${workDir}/deny`, { force: true });
    }
    await click(button('Discover'));
    await click(`${card('renamed')}${button('Install')}`);
    await waitFor('unrelated install completes', () => existsSync(`${workDir}/pkgs/renamed`), 30000);
    await click(nav('Activity'));
    await click(button('Retry homebrew'));
    await click(button('Discover'));
    await waitFor('homebrew installed', () => find(`${card('homebrew')}//*[normalize-space()='Installed']`), 30000);
    await waitFor('brewed installed', () => find(`${card('brewed')}//*[normalize-space()='Installed']`), 30000);
    await waitFor('brew-only installable now', () => find(`${card('brew-only')}${button('Install')}`));
  }],
  ['a program installed outside furnish shows as installed there, without Uninstall', async () => {
    await waitFor('elsewhere marked', () => find(`${card('elsewhere')}//*[normalize-space()='Installed outside furnish']`));
    await click(`${card('elsewhere')}//button[@aria-label='Open elsewhere']`);
    await waitFor('where its program is', async () => (await text('//section')).includes('/bin/elsewhere-tool'));
    await waitFor('its version can\'t be checked', async () => (await text('//section')).includes("Acme Apps can't check this app's version against the versions fixture requires: it was installed outside Acme Apps, at "));
    if ((await findAll(button('Uninstall'))).length) throw new Error('offers Uninstall');
    await click(button('Discover'));
  }],
  ['no acceptable version offered: the card and the app page say what is needed and what pacman offers', async () => {
    const needs = 'Needs 2.0 or later; pacman offers 1.0';
    await waitFor('outdated shows what it needs', async () => (await text(card('outdated'))).includes(needs), 30000);
    if ((await findAll(`${card('outdated')}${button('Install')}`)).length) throw new Error('offers Install without an acceptable version');
    await wd('POST', s('/execute/sync'), { script: "arguments[0].scrollIntoView({ block: 'center' });", args: [{ [ELEMENT]: await find(card('outdated')) }] });
    await snap('required-unmet-card');
    await click(`${card('outdated')}//button[@aria-label='Open outdated']`);
    await waitFor('status line', async () => (await text("//p[contains(@class,'status')]")) === needs);
    await snap('required-unmet-page');
    await click(button('Discover'));
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
  ['"What this sets up" lists only what applies here, by name', async () => {
    const list = "//h2[normalize-space()='What this sets up']/following-sibling::ul[1]";
    const items = await findAll(`${list}/li`);
    if (items.length !== 1 || (await text(`${list}/li`)).trim() !== 'hyperfine') throw new Error(`expected only the Linux resource: ${await text(list)}`);
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
    if (!(await findAll(`${panel}//li[contains(@class,'skipped') and contains(., 'hyperfine (winget)')]`)).length) throw new Error('skipped Windows step missing');
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
  ['Activity keeps the exact steps and outcome of a completed run', async () => {
    await click(nav('Activity'));
    const extras = "//li[contains(@class,'history')][.//*[normalize-space()='Installed extras']]";
    await waitFor('completed install in Activity', () => find(extras));
    if (!(await text(extras)).includes('Started by you')) throw new Error('install trigger missing from Activity');
    await click(`${extras}${button('Steps')}`);
    await waitFor('recorded package command in Activity', async () => {
      const step = await text(`${extras}//ol[contains(@class,'steps')]/li`);
      return step.includes('pacman -S') && step.includes('extras') && step.includes('Installed');
    });
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
  ['uninstall confirmation names the installed app and kit that require the selection', async () => {
    await click(button('Discover'));
    await click(`${card('hyperfine')}//button[@aria-label='Open hyperfine']`);
    await click(button('Uninstall'));
    const impact = "//div[contains(@class,'impact')]";
    await waitFor('app and kit impact', async () => {
      const message = await text(impact);
      return message.includes('devtool') && message.includes('starter');
    });
    await click(button('Cancel'));
    if (!existsSync(`${workDir}/pkgs/hyperfine`)) throw new Error('cancel removed the prerequisite');
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
    await click(nav('Settings'));
    await click("//label[contains(., 'Wait while this computer is busy')]//input");
    await waitFor('setting saved', () => log('data/settings.json').includes('"loadAwareUpdates": false'));
    await waitFor('hyperfine updated now', () => log('pkgs/hyperfine').trim() === '3.0', 30000);
  }],
  ['a security update the collection requires installs right away, also with automatic updates off', async () => {
    await click(button('Discover'));
    await click(`${card('secure')}//button[@aria-label='Open secure']`);
    await click("//label[contains(., 'Update automatically')]/input");
    await waitFor('opt-out saved', () => log('data/settings.json').includes('fixture/secure'));
    writeFileSync(`${workDir}/pkgs/secure.new`, '1.6\n');
    // The collection raises its requirement above the installed 1.0 (e.g. after a security advisory).
    const git = (...args) => {
      const r = spawnSync('git', ['-C', `${workDir}/origin-src`, ...args], { encoding: 'utf8' });
      if (r.status) throw new Error(`git ${args.join(' ')}: ${r.stderr}`);
    };
    const yaml = `${workDir}/origin-src/apps/secure.dsc.yaml`;
    writeFileSync(yaml, readFileSync(yaml, 'utf8').replace("'>=1.0'", "'>=1.5'"));
    git('-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-qam', 'secure: 1.5 or later');
    git('push', '-q', `${workDir}/origin.git`, 'HEAD');
    // The first attempt is refused (password dialog): it must not try again on its own.
    writeFileSync(`${workDir}/deny`, '');
    try {
      await click("//nav//button[starts-with(normalize-space(), 'Updates')]");
      await click(button('Check for updates'));
      await waitFor('started without a click', () => find("//div[contains(@class,'toast') and contains(., 'Installing a required security update for secure')]"), 30000);
      await waitFor('first attempt refused', () => log('denied.log').includes('secure'), 30000);
      await waitFor('marked on the card', async () => (await text(card('secure'))).includes('Security update required'), 30000);
      await sleep(5000);
      const attempts = log('denied.log').split('\n').filter((l) => l.includes('secure')).length;
      if (attempts !== 1) throw new Error(`${attempts} attempts after a refused one, expected no retry until the next check`);
      await snap('required-update-card');
      await click(`${card('secure')}//button[@aria-label='Open secure']`);
      await waitFor('status line', async () => (await text("//p[contains(@class,'status')]")) === 'Installed, security update required');
      const page = await text("//section[contains(@class,'detail')]");
      if (!page.includes('fixture requires 1.5 or later. Version 1.0 is installed, so 1.6 installs right away.')) throw new Error(`detail: ${page}`);
      if (!page.includes('fixture requires this update, so it installs even with automatic updates off.')) throw new Error(`toggle note: ${page}`);
      await snap('required-update-page');
    } finally {
      rmSync(`${workDir}/deny`);
    }
    if (log('pkgs/secure').trim() !== '1.0') throw new Error('installed while refused');
    // The next check tries again, although automatic updates are off and the last attempt was declined.
    await click("//nav//button[starts-with(normalize-space(), 'Updates')]");
    await click(button('Check for updates'));
    await waitFor('secure updated to 1.6', () => log('pkgs/secure').trim() === '1.6', 30000);
    await click(button('Discover'));
    await waitFor('secure installed', () => find(`${card('secure')}//*[normalize-space()='Installed']`), 30000);
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
    const change = "//table[contains(@class,'changes')]//tr[th[normalize-space()='Install just for me']]";
    await waitFor('installed and proposed values', async () =>
      (await text(`${change}/td[1]`)) === 'Yes' && (await text(`${change}/td[2]`)).includes('App setting')
        && (await text(`${change}/td[2]`)).startsWith('No'));
    await choose('Same as the collection');
    await waitFor('inherited proposal attributed to its collection', async () =>
      (await text(`${change}/td[2]`)).includes('From collection fixture'));
    // Its saved user install still requires Homebrew until this reinstall applies the change.
    await click(button('Discover'));
    await click(`${card('homebrew')}//button[@aria-label='Open homebrew']`);
    await click(button('Uninstall'));
    await waitFor('impact uses installed options', async () => (await text("//div[contains(@class,'impact')]")).includes('cli-tool'));
    await click(button('Cancel'));
    await click(button('Discover'));
    await click(`${card('cli-tool')}//button[@aria-label='Open cli-tool']`);
    await click(button('Reinstall with these options'));
    await waitFor('moved to a system-wide install', () => existsSync(`${workDir}/pkgs/cli-tool`) && !existsSync(`${workDir}/brew-pkgs/cli-tool`), 30000);
    await waitFor('banner gone', async () => !(await findAll(button('Reinstall with these options'))).length);
  }],
  ['collection options apply to all its apps', async () => {
    await click(nav('Collections'));
    await click("//summary[contains(., 'Options for all fixture apps')]");
    await waitFor('collection option shown', async () => (await text("//details[contains(@class,'copts')]")).includes('Install just for me'));
  }],
  ['collections: add a repository, see its apps, remove it again', async () => {
    await click(nav('Collections'));
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
    await click(nav('Settings'));
    await click("//label[contains(., 'When Acme Apps starts')]/input");
    await waitFor('settings.json', () => log('data/settings.json').includes('"updateOnLaunch": false'));
    // The admin policy locks the interval.
    if (!(await prop("//select[@id='interval']", 'disabled'))) throw new Error('locked interval is editable');
    if (!(await text("//label[contains(@class,'interval')]")).includes('Set by your organization')) throw new Error('no hint');
    if (log('data/settings.json').includes('updateIntervalMinutes')) throw new Error('locked setting saved for the user');
  }],
  ['a collection update that is not applied yet is marked; updating clears the mark', async () => {
    // The server moves on; with updating on launch off (the step before), furnish only asks it.
    const git = (...args) => {
      const r = spawnSync('git', ['-C', `${workDir}/origin-src`, ...args], { encoding: 'utf8' });
      if (r.status) throw new Error(`git ${args.join(' ')}: ${r.stderr}`);
    };
    writeFileSync(`${workDir}/origin-src/apps/later.dsc.yaml`,
      'metadata: { furnish: { description: Arrived later, categories: [Test] } }\nresources:\n  - name: later\n    type: Furnish/Package\n    properties: { name: later }\n');
    git('add', '-A');
    git('-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-qm', 'later');
    git('push', '-q', `${workDir}/origin.git`, 'HEAD');
    await startSession();
    const badge = `${nav('Collections')}//span[@aria-label='1 collection has an update']`;
    await waitFor('indicator in the sidebar', () => find(badge), 30000);
    if ((await findAll(card('later'))).length) throw new Error('the update was applied, not only found');
    await click(nav('Collections'));
    const row = "//li[.//*[normalize-space()='fixture']]";
    await waitFor('collection marked', async () => (await text(row)).includes('Update available'));
    await click(`${row}//button[normalize-space()='Update']`);
    await waitFor('indicator gone', async () => !(await findAll(badge)).length, 30000);
    if ((await text(row)).includes('Update available')) throw new Error('still marked after updating');
    await click(nav('Discover'));
    await waitFor('the update brought its app', () => find(card('later')), 30000);
  }],
  ['a failed install offers the earlier version; installing it holds the app there', async () => {
    await click(button('Discover'));
    await click(`${card('flaky')}${button('Install')}`);
    await click(`${card('flaky')}//button[@aria-label='Open flaky']`);
    await waitFor('offer of the earlier version', () => find("//button[starts-with(normalize-space(), 'Install the version from')]"), 30000);
    await click("//button[starts-with(normalize-space(), 'Install the version from')]");
    await waitFor('held notice', async () => (await text('//section')).includes("installing the collection's current version failed"), 30000);
    await waitFor('installed', async () => (await text("//p[contains(@class,'status')]")) === 'Installed and up to date');
    if (!existsSync(`${workDir}/pkgs/flaky`)) throw new Error('the earlier version did not install package flaky');
    if (!(await findAll(button('Try the current version'))).length) throw new Error('no way back to the current version');
  }],
  ['per-collection settings: falling back, downloads, history', async () => {
    // Error notifications stay until dismissed; the one from the failed install would cover the list.
    for (const el of await findAll("//button[@aria-label='Dismiss']")) await wd('POST', s(`/element/${el}/click`), {});
    await click(nav('Collections'));
    await click("//li[.//*[normalize-space()='fixture']]//summary[contains(., 'Downloads and earlier versions')]");
    await click("//select[@id='cs-fixture-fallback']/option[normalize-space()='Install the previous version']");
    await waitFor('saved for the collection', () => /"fixture":\s*\{\s*"fallback":\s*"automatic"/.test(log('data/settings.json')));
  }],
  ['collections the organization provides cannot be removed', async () => {
    const row = "//li[.//*[normalize-space()='fixture']]";
    if (!(await text(row)).includes('Managed by your organization')) throw new Error('not marked managed');
    if ((await findAll(`${row}//button[normalize-space()='Remove']`)).length) throw new Error('Remove offered');
  }],
  ["reports: the organization's setting shows and can't be changed", async () => {
    await click(nav('Settings'));
    const section = "//section[h2[normalize-space()='Reports to your organization']]";
    await waitFor('reports section', () => find(section));
    if (!(await text(section)).includes('Set by your organization')) throw new Error('no hint that the organization sets it');
    if (!(await prop(`${section}//select`, 'disabled'))) throw new Error('locked reporting setting is editable');
    if ((await prop(`${section}//select`, 'value')) !== 'errors') throw new Error('not the policy value');
  }],
  ['the Settings page shows the version and opens the open-source licenses', async () => {
    await click(nav('Settings'));
    await waitFor('version shown', async () => /furnish \d+\.\d+\.\d+/.test(await text("//*[@id='version']")));
    await click(button('Open-source licenses'));
    await waitFor('licenses opened', () => log('browser.log').includes('licenses.html') || log('browser.log').includes('THIRD-PARTY-LICENSES.html'));
  }],
  ['a newer furnish shows as a dot on Settings and in About, with how it arrives', async () => {
    await waitFor('dot on Settings', () => find("//nav//button[starts-with(normalize-space(), 'Settings')]//*[@aria-label='A new version of Acme Apps is available']"), 30000);
    await click(nav('Settings'));
    const row = "//*[@id='self-update']";
    await waitFor('the new version in About', async () => (await text(row)).includes('Version 99.0.0 is available.'));
    if (!(await text(row)).includes('Install it the way you installed Acme Apps.')) throw new Error(`how it arrives: ${await text(row)}`);
    // Built from source: furnish doesn't install it, and doesn't offer to.
    if ((await findAll(`${row}//button`)).length) throw new Error('offers to install a version it cannot install here');
    const options = await wd('POST', s('/execute/sync'), {
      script: "return [...document.querySelectorAll('#self-update-mode option')].map((o) => o.value)", args: [],
    });
    if (JSON.stringify(options) !== '["notify","off"]') throw new Error(`choices: ${JSON.stringify(options)}`);
    await click("//select[@id='self-update-mode']/option[@value='off']");
    await waitFor('off saved', () => log('data/settings.json').includes('"selfUpdate": "off"'));
    await waitFor('dot gone', async () => !(await findAll("//nav//*[@aria-label='A new version of Acme Apps is available']")).length);
    await click("//select[@id='self-update-mode']/option[@value='notify']");
    await waitFor('notify saved', () => log('data/settings.json').includes('"selfUpdate": "notify"'));
  }],
  ['the language setting switches to German and back, without a restart', async () => {
    await click(nav('Settings'));
    const englishWidth = await wd('POST', s('/execute/sync'), {
      script: "return parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--action-width'))", args: [],
    });
    if (!(englishWidth > 0)) throw new Error(`--action-width in English: ${englishWidth}`);
    await click("//select[@id='language']/option[@value='de']");
    await waitFor('German saved', () => log('data/settings.json').includes('"language": "de"'));
    await waitFor('German Settings page', async () => (await text('//h1')) === 'Einstellungen');
    await find(button('Jetzt prüfen'));
    await click(button('Entdecken'));
    await waitFor('German Discover page', async () => (await text('//h1')) === 'Entdecken');
    await find("//input[@placeholder='Apps durchsuchen']");
    await find(`${card('fresh')}${button('Installieren')}`);
    // The collection's German texts (tests/fixtures/collection/locales/de.yaml), without a restart.
    await waitFor('German app name and description', () => find(`${card('Werkzeugkiste')}//p[normalize-space()='Zwei Kommandozeilenwerkzeuge']`));
    await click("//nav//button[normalize-space()='Büro']");
    await waitFor('German category page', async () => (await text('//h1')) === 'Büro');
    await find(card('Werkzeugkiste'));
    await click(button('Entdecken'));
    // Search ignores accents, and also finds the collection's English texts (the fallback language).
    const search = (value) => wd('POST', s('/execute/sync'), {
      script: "const i = document.querySelector('input.search'); i.value = arguments[0]; i.dispatchEvent(new Event('input', { bubbles: true }));",
      args: [value],
    });
    const shown = async (app) => (await findAll(card(app))).length > 0;
    await search('buro');
    await waitFor('"buro" finds Büro', async () => (await shown('Werkzeugkiste')) && !(await shown('hyperfine')));
    await search('two command');
    await waitFor('English text found', async () => (await shown('Werkzeugkiste')) && !(await shown('hyperfine')));
    await search('');
    await waitFor('all apps again', () => shown('hyperfine'));
    // The install button widens to fit the longest German label ("Erst beantworten").
    const width = () => wd('POST', s('/execute/sync'), {
      script: "return parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--action-width'))", args: [],
    });
    await waitFor('German button width', async () => (await width()) > englishWidth);
    await click(nav('Einstellungen'));
    await click("//select[@id='language']/option[@value='en']");
    await waitFor('English again', async () => (await text('//h1')) === 'Settings');
    await waitFor('English saved', () => log('data/settings.json').includes('"language": "en"'));
  }],
  ['the logo leads back to Discover', async () => {
    await click("//button[@aria-label='Acme Apps, go to Discover']");
    await waitFor('Discover page', async () => (await text('//h1')) === 'Discover');
  }],
  ['starting at sign-in: on by default, the setting adds and removes the entry', async () => {
    await waitFor('sign-in entry written at start', () => autostartEntry().includes('--hidden'));
    if (!/^Exec=\/usr\/local\/bin\/furnish-gui --hidden$/m.test(autostartEntry())) throw new Error(`entry: ${autostartEntry()}`);
    await click(nav('Settings'));
    const box = "//label[contains(., 'Start Acme Apps when you sign in')]//input";
    if (!(await prop(box, 'checked'))) throw new Error('off by default');
    await click(box);
    await waitFor('turned off', () => log('data/settings.json').includes('"startAtSignIn": false'));
    await waitFor('entry removed', () => !autostartEntry());
    await click(box);
    await waitFor('turned on', () => log('data/settings.json').includes('"startAtSignIn": true'));
    await waitFor('entry back', () => autostartEntry().includes('--hidden'));
  }],
  ['the organization can lock starting at sign-in; furnish applies it when it starts', async () => {
    const policy = log('policy.json');
    const locked = JSON.parse(policy);
    locked.settings.startAtSignIn = { value: false, locked: true };
    writeFileSync(`${workDir}/policy.json`, JSON.stringify(locked));
    try {
      await startSession();
      await waitFor('entry removed at start', () => !autostartEntry());
      await click(nav('Activity'));
      await waitFor('history after restart', () => find("//li[contains(@class,'history')]//time"));
      await click(nav('Settings'));
      const label = "//label[contains(., 'Start Acme Apps when you sign in')]";
      await waitFor('locked', async () => (await prop(`${label}//input`, 'disabled')) && !(await prop(`${label}//input`, 'checked')));
      if (!(await text(label)).includes('Set by your organization')) throw new Error('no hint that the organization sets it');
    } finally {
      writeFileSync(`${workDir}/policy.json`, policy);
    }
  }],
  ['Activity groups a partial kit install and retries the failed member', async () => {
    await click(button('Discover'));
    await click("//button[@aria-label='Open kit recovery']");
    writeFileSync(`${workDir}/fail-once-toolbox-b`, '');
    await click(button('Install 2 apps'));
    await click(nav('Activity'));
    const group = "//li[contains(@class,'kit-history')][.//summary[contains(., 'recovery')]]";
    await waitFor('one of two installed', async () => (await text(`${group}//summary`)).includes('1 of 2 installed'), 30000);
    await click(`${group}//summary`);
    const retry = `${group}${button('Retry toolbox')}`;
    await waitFor('toolbox failure is actionable', () => find(retry));
    const freshInstalls = () => log('pkexec.log').split('\n').filter(line => line.includes('pacman -S') && line.endsWith('fresh')).length;
    const before = freshInstalls();
    if (before !== 1) throw new Error(`fresh installs before retry: ${before}`);
    await click(retry);
    await waitFor('same kit request completes', async () => (await text(`${group}//summary`)).includes('2 of 2 installed'), 30000);
    if ((await findAll(group)).length !== 1 || freshInstalls() !== before) throw new Error('retry duplicated the kit or its successful app');
    await startSession();
    await click(nav('Activity'));
    await waitFor('kit result survives a restart', async () => (await text(`${group}//summary`)).includes('2 of 2 installed'));
  }],
  ['a Nix installation shows its flake update instructions and skips the release feed', async () => {
    const requests = () => (log('feed.log').match(/GET \/latest\.json /g) ?? []).length;
    // The regular install has checked this feed, so this also verifies that the counter is live.
    await waitFor('release request from the regular install', () => requests() > 0);
    await wd('DELETE', s(''));
    session = null;
    const before = requests();

    await startSession('/nix/store/fixture-furnish/bin/furnish-gui');
    await click(nav('Settings'));
    await waitFor('flake update guidance in About', () => find("//section[@aria-labelledby='about-title']//code[.='nix flake update']"));
    if ((await findAll("//*[@id='self-update-mode']")).length) throw new Error('Nix installs offer a release update setting');

    // Exercise the same native command the launch and interval checks use, and await its answer.
    const status = await wd('POST', s('/execute/async'), {
      script: "const done = arguments[arguments.length - 1]; window.__TAURI_INTERNALS__.invoke('check_self_update', { download: false }).then(done, e => done(String(e)))",
      args: [],
    });
    if (status.method !== 'nix' || status.mode !== 'off' || status.available || requests() !== before) {
      throw new Error(`Nix updater: ${JSON.stringify(status)}, ${requests() - before} release requests`);
    }
  }],
];

let failed = false;
try {
  await startSession();
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
