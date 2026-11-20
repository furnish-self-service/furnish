// UI tests for the real furnish GUI, driven over WebDriver (tauri-driver on localhost, port DRIVER_PORT).
// Started by tests/ui/run.sh, which prepares the app's data dir and a fake package manager.
// Plain fetch, no test framework: each step names what it checks, the first failure stops the run.
import { readFileSync, existsSync, writeFileSync, rmSync, mkdirSync } from 'node:fs';
import { spawnSync } from 'node:child_process';

// run.sh picks the port, so runs side by side (host network) don't meet each other's driver.
const DRIVER = `http://127.0.0.1:${process.env.DRIVER_PORT || 4444}`;
const [workDir, screenshot, container, replaceDesktop] = process.argv.slice(2);
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
// WebKit can report a successful click on a card that moved under the pointer during a
// catalog scan. Guard actual pointer delivery before any neighboring control can act.
function guardClick(target) {
  target.scrollIntoView({ block: 'center', inline: 'center' });
  // WebDriver selects an option through its native select control.
  if (target.tagName === 'OPTION') return false;
  let result = target.matches(':disabled') ? 'control is disabled' : 'no click reached the document';
  const guard = (event) => {
    if (event.composedPath().includes(target)) {
      result = true;
    } else {
      event.preventDefault();
      event.stopImmediatePropagation();
      const other = event.target.closest('button, a, summary') ?? event.target;
      const label = other.getAttribute('aria-label') || other.textContent.trim().slice(0, 60);
      result = `click landed on ${other.tagName} ${label}`.trim();
    }
  };
  window.addEventListener('click', guard, { capture: true, once: true });
  window.finishFurnishTestClick = () => {
    window.removeEventListener('click', guard, true);
    delete window.finishFurnishTestClick;
    return result;
  };
  return true;
}

async function click(xpath) {
  const result = await waitFor(`click on ${xpath}`, async () => {
    const el = await find(xpath);
    const guarded = await wd('POST', s('/execute/sync'), {
      script: `return (${guardClick})(arguments[0])`, args: [{ [ELEMENT]: el }],
    });
    const error = await wd('POST', s(`/element/${el}/click`), {}).then(() => null, e => e);
    if (!guarded) {
      if (error) throw error;
      return true;
    }
    // A dispatched click may already have changed the app. If delivery cannot be read
    // back, fail this test instead of sending the action twice.
    const hit = await wd('POST', s('/execute/sync'), { script: 'return window.finishFurnishTestClick()', args: [] }).catch(e => e);
    if (hit === true || hit instanceof Error) return hit;
    throw new Error(error ? `${hit}: ${error.message}` : hit);
  });
  if (result instanceof Error) throw result;
}
const text = async (xpath) => wd('GET', s(`/element/${await find(xpath)}/text`));
const typeInto = async (xpath, value) => wd('POST', s(`/element/${await find(xpath)}/value`), { text: value });
const search = (value) => wd('POST', s('/execute/sync'), {
  script: "const i = document.querySelector('input.search'); i.value = arguments[0]; i.dispatchEvent(new Event('input', { bubbles: true }));",
  args: [value],
});
// WebKitWebDriver reports content of a closed <details> as displayed, so check `open` instead.
const prop = async (xpath, name) => wd('GET', s(`/element/${await find(xpath)}/property/${name}`));
const isOpen = async (xpath) => wd('GET', s(`/element/${await find(xpath)}/property/open`));
const button = (label) => `//button[normalize-space()='${label}']`;
// A sidebar entry; Collections may carry a count after its name.
const nav = (label) => `//nav//button[starts-with(normalize-space(), '${label}')]`;

async function setLoadAwareUpdates(enabled) {
  await click(nav('Settings'));
  const toggle = "//label[contains(., 'Wait while this computer is busy')]//input";
  await waitFor('load-aware update setting', () => find(toggle));
  if (await prop(toggle, 'checked') !== enabled) await click(toggle);
  await waitFor(`load-aware updates ${enabled ? 'enabled' : 'disabled'}`,
    () => (JSON.parse(log('data/settings.json')).loadAwareUpdates ?? true) === enabled);
}
const card = (app) => `//article[.//h3[normalize-space()='${app}']]`;
// Extra screenshots of states worth seeing, next to the final one.
const snap = async (name) =>
  writeFileSync(screenshot.replace(/\.png$/, `-${name}.png`), Buffer.from(await wd('GET', s('/screenshot')), 'base64'));
const log = (file) => (existsSync(`${workDir}/${file}`) ? readFileSync(`${workDir}/${file}`, 'utf8') : '');
// The sign-in entry furnish writes (HOME=/tmp in the container).
const LAUNCHER = 'user-share/applications/furnish.desktop';
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

function settingsWriter(start) {
  const result = spawnSync('docker', ['exec', ...(start ? ['-d'] : []), container, 'sh', '-c', start
    ? 'cp /bin/sleep "$1/furnish-ui-cache"; echo $$ > "$1/writer.pid"; exec "$1/furnish-ui-cache" 300'
    : 'kill "$(cat "$1/writer.pid")"', 'sh', workDir], {encoding: 'utf8'});
  if (result.status !== 0) throw new Error(result.stderr);
}

function inContainer(args, root = false) {
  const result = spawnSync('docker', ['exec', ...(root ? ['--user', '0'] : []), container, ...args], {encoding: 'utf8'});
  if (result.status !== 0) throw new Error(result.stderr || `docker exec failed with ${result.status}`);
  return result.stdout.trim();
}

const steps = [
  ['Steps previews the NixOS resource in the desktop app', async () => {
    await click(`${card('nixos-preview')}//button[@aria-label='Open nixos-preview']`);
    const panel = "//details[summary[contains(., 'Steps')]]";
    await click(`${panel}/summary`);
    await waitFor('NixOS removal preview', async () => (await text(panel)).includes('change nothing'));
    await click(`${panel}//button[normalize-space()='Install']`);
    await waitFor('NixOS install preview', async () => (await text(panel)).includes('Installing nixos-preview'));
    await click(nav('Discover'));
  }],
  ['a session-changing install explains when to sign out in its preview and completed results', async () => {
    const panel = "//details[summary[contains(., 'Steps')]]";
    await click(`${card('session-notice')}//button[@aria-label='Open session-notice']`);
    if (!(await isOpen(panel))) await click(`${panel}/summary`);
    await waitFor('login notice in install preview', async () => (await text(panel)).includes('After installation, sign out of your computer'));
    await click("//button[contains(@class,'action') and normalize-space()='Install']");
    await waitFor('session setup wrote its file', () => log('session-notice') === 'ready');
    const after = 'Sign out of your computer and sign back in to use these changes.';
    await waitFor('completed plan gives the login instruction', async () => (await text(panel)).includes(after));
    await click(`${panel}/summary`);
    await waitFor('login instruction remains visible with expert Steps closed', async () => (await text("//p[@role='status']")).includes(after));
    await click(nav('Activity'));
    const history = "//li[contains(@class,'history')][.//*[normalize-space()='Installed session-notice']]";
    await waitFor('Activity retains the login instruction', async () => (await text(history)).includes(after));
    await click(nav('Discover'));
  }],
  ['previewing a collection update shows how an installed app would change', async () => {
    const git = (dir, ...args) => {
      const result = spawnSync('git', ['-C', `${workDir}/${dir}`, ...args], { encoding: 'utf8' });
      if (result.status !== 0) throw new Error(result.stderr);
      return result.stdout.trim();
    };
    const checkout = 'data/collections/fixture';
    const current = git(checkout, 'rev-parse', 'HEAD');
    const appPath = 'apps/session-notice.dsc.yaml';
    const definition = readFileSync(`${workDir}/origin-src/${appPath}`, 'utf8');
    writeFileSync(`${workDir}/origin-src/${appPath}`, definition.replace('name: Session setup', 'name: Refreshed session setup'));
    git('origin-src', '-c', 'user.name=t', '-c', 'user.email=t@t', 'commit', '-qm', 'Rename the installed session setup step', '--', appPath);
    const candidate = git('origin-src', 'rev-parse', 'HEAD');
    git('origin-src', 'tag', 'release-preview-test', candidate);
    git('origin-src', 'push', '-q', `${workDir}/origin.git`, 'HEAD', 'refs/tags/release-preview-test');

    await click(nav('Collections'));
    const panel = "//details[@data-release-preview='fixture']";
    await click(`${panel}/summary`);
    await waitFor('current and candidate releases and their changed steps', async () => {
      const preview = await text(panel);
      const expected = [current.slice(0, 7), candidate.slice(0, 7), 'release-preview-test', 'session-notice',
        'Recorded as installed', 'Removed · Session setup', 'Added · Refreshed session setup'];
      return expected.every((part) => preview.includes(part));
    }, 30000);
    if (git(checkout, 'rev-parse', 'HEAD') !== current || log('session-notice') !== 'ready') {
      throw new Error('opening a preview changed the checkout or its installed app');
    }
    await snap('collection-release-preview');
    await click(`${panel}//button[normalize-space()='Apply reviewed revision']`);
    await waitFor('applying the reviewed revision', () => git(checkout, 'rev-parse', 'HEAD') === candidate);
    await click(nav('Discover'));
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
    await startSession();
  }],
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
  ['the Linux launcher uses the organization name and icon with the package command and actions', async () => {
    const entry = await waitFor('branded package launcher', () => {
      const entry = log(LAUNCHER);
      return entry.includes('Name=Acme Apps\n') && entry;
    });
    for (const line of ['Exec=/usr/local/bin/furnish-gui %u', 'MimeType=x-scheme-handler/furnish;', 'StartupWMClass=furnish', 'Actions=Hidden;', '[Desktop Action Hidden]', 'Exec=/usr/local/bin/furnish-gui --hidden']) {
      if (!entry.split('\n').includes(line)) throw new Error(`missing package entry line: ${line}`);
    }
    if (entry.includes('Name[de]=Paketverwaltung')) throw new Error('the localized package name shadows the branding');
    const icon = entry.match(/^Icon=(.+)$/m)?.[1];
    if (!icon?.startsWith(`${workDir}/user-share/`) || readFileSync(icon).subarray(1, 4).toString() !== 'PNG') {
      throw new Error(`launcher icon is not a PNG in the user data directory: ${icon}`);
    }
  }],
  ['Discover recommends apps for the role; cards show the collection avatar', async () => {
    // The policy answers the fixture collection's role question: developer.
    const row = "//div[contains(@class,'grid') and contains(@class,'recommended')]";
    // By the cards' headings in the DOM (WebKit's element text leaves them out).
    const inRow = (app) => findAll(`${row}//h3[normalize-space()='${app}']`).then((l) => l.length > 0);
    await waitFor('starter, hyperfine and devtool recommended', async () => (await inRow('starter')) && (await inRow('hyperfine')) && (await inRow('devtool')), 30000);
    if ((await findAll(card('hyperfine'))).length !== 1) throw new Error('recommended app repeated in the catalog');
    if (await inRow('toolbox')) throw new Error('toolbox recommended');
    // Avatars come with the collection list, which loads separately (and again after the update on launch).
    await waitFor('collection avatar on cards', async () => (await findAll("//article//span[contains(@class,'collection')]//img")).length > 0);
    // The fixture's iconDark: offered to dark mode beside the avatar, as an image of its own.
    const dark = "//article//span[contains(@class,'collection')]//picture/source[@media='(prefers-color-scheme: dark)'][starts-with(@srcset,'data:image/svg+xml;base64,')]";
    await waitFor('dark-mode avatar on the cards', async () => (await findAll(dark)).length > 0);
  }],
  ['new destinations open at the top and Back restores the Settings position', async () => {
    const scrollTop = () => prop('//main', 'scrollTop');
    await click(nav('Settings'));
    await click("//a[@href='#about-title']");
    await waitFor('jump to About', async () => (await scrollTop()) > 0);
    const previous = await scrollTop();
    await click(nav('Discover'));
    await waitFor('new destination starts at its heading', async () => (await scrollTop()) === 0);
    await wd('POST', s('/execute/sync'), {
      script: "window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft', altKey: true }))", args: [],
    });
    await waitFor('Back restores the scrolled Settings page', async () =>
      (await text('//h1')) === 'Settings' && Math.abs((await scrollTop()) - previous) <= 1);
    await click(nav('Discover'));
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
    // Removing the finished operation's Now row can move Retry between mouse down and up.
    // Wait for the desktop to finish, including its status refresh.
    await waitFor('renamed installed and no longer busy', () =>
      find(`${card('renamed')}[not(.//*[@aria-busy='true'])]//*[normalize-space()='Installed']`), 30000);
    await click(nav('Activity'));
    await click(button('Retry homebrew'));
    await waitFor('Activity accepts the prerequisite retry', async () => !(await findAll(button('Retry homebrew'))).length);
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
    await waitFor('the preview explains the selected manager and package name', async () => {
      const plan = await text(panel);
      return plan.includes('pacman is the first available system package manager') && plan.includes('hyperfine is the default package name');
    });
    if (log('pkexec.log').includes('-R')) throw new Error('the preview ran the removal');
    await click(`${panel}//button[normalize-space()='Install']`);
    await waitFor('install preview: nothing to do', async () => (await text(panel)).includes('change nothing'));
    const skipped = `${panel}//li[contains(@class,'skipped')]`;
    await waitFor('skipped step explains the condition in prose', async () => (await text(`${skipped}//p`)).includes('Only on Windows'));
    const condition = await text(`${skipped}//code[contains(@class,'condition')]`);
    if (condition !== "[equals(context().os.family, 'Windows')]") throw new Error(`incomplete DSC condition: ${condition}`);
    await wd('POST', s('/execute/sync'), { script: "arguments[0].scrollIntoView({ block: 'center' })", args: [{ [ELEMENT]: await find(skipped) }] });
    await snap('steps');
    const overflow = await wd('POST', s('/execute/sync'), {
      script: `const plan = arguments[0].closest('.plan');
        const width = plan.style.width;
        plan.style.width = '21rem';
        try {
          return [...plan.querySelectorAll('.steps li')].some(row => row.scrollWidth > row.clientWidth + 1);
        } finally { plan.style.width = width; }`,
      args: [{ [ELEMENT]: await find(skipped) }],
    });
    if (overflow) throw new Error('the narrow Steps preview overflows its rows');
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
    const current = `${panel}//li[contains(concat(' ', normalize-space(@class), ' '), ' running ')]`;
    await waitFor('a numbered running step', async () => {
      const items = await findAll(`${panel}//ol[contains(@class, 'steps')]/li`);
      const n = items.indexOf(await find(current)) + 1;
      return n > 0 && (await text(`${current}/span[contains(@class, 'marker')]`)).trim() === String(n);
    });
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
    await setLoadAwareUpdates(false);
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
    await setLoadAwareUpdates(true);
    // Real CPU load from outside furnish: a busy loop on every core for 45 s.
    spawnSync('docker', ['exec', '-d', container, 'sh', '-c',
      'for i in $(seq $(nproc)); do timeout 45 sh -c "while :; do :; done" & done; wait']);
    await sleep(20000); // the 30 s test window fills with busy samples
    writeFileSync(`${workDir}/pkgs/hyperfine.new`, '3.0\n');
    await click("//nav//button[starts-with(normalize-space(), 'Updates')]");
    await click(button('Check for updates'));
    await waitFor('update postponed', async () => (await text("//*[contains(@class,'postponed')]")).includes('busy (CPU'), 20000);
    if (log('pkgs/hyperfine').trim() === '3.0') throw new Error('updated although the computer was busy');
    await setLoadAwareUpdates(false);
    await waitFor('hyperfine updated now', () => log('pkgs/hyperfine').trim() === '3.0', 30000);
  }],
  ['Update all queues updates hidden by search and respects automatic-update preferences', async () => {
    await click(nav('Discover'));
    await click(`${card('hyperfine')}//button[@aria-label='Open hyperfine']`);
    await waitFor('previous update finished', () => find(button('Uninstall')));
    await click("//label[contains(., 'Update automatically')]/input");
    const optOuts = () => JSON.parse(log('data/settings.json')).noAutoUpdate;
    await waitFor('both automatic updates turned off', () =>
      ['fixture/hyperfine', 'fixture/extras'].every((key) => optOuts().includes(key)));
    writeFileSync(`${workDir}/pkgs/hyperfine.new`, '4.0\n');
    await click(nav('Updates'));
    await click(button('Check for updates'));
    for (const app of ['hyperfine', 'extras']) await waitFor(`${app} update available`, () => find(`${card(app)}${button('Update')}`));

    await search('extras');
    await waitFor('only extras visible', async () => (await findAll(card('extras'))).length && !(await findAll(card('hyperfine'))).length);
    writeFileSync(`${workDir}/fail-once-extras`, '');
    const updateAll = button('Update all');
    await click(`${updateAll}[not(@disabled)]`);
    await waitFor('pending batch cannot be queued twice', () => prop(updateAll, 'disabled'));
    await search('');
    await click(nav('Activity'));
    const group = "//li[@data-kind='updates']";
    const summary = `${group}/details/summary`;
    await waitFor('partial Update all result', async () => (await text(summary)).includes('1 of 2 updated') && log('pkgs/hyperfine').trim() === '4.0', 30000);
    await startSession();
    // The next scenario changes a collection during a manual check. Finish the separate
    // startup check before measuring how many times that manual check requests elevation.
    await click(nav('Updates'));
    await waitFor('startup check finished', () => find("//div[contains(@class,'check')]//span[contains(., 'Last checked ')]"), 30000);
    await click(nav('Activity'));
    await click(summary);
    await waitFor('manual trigger recorded on the group', async () => (await text(group)).includes('Started by you'));
    const extras = `${group}//li[@data-app='fixture/extras']`;
    await click(`${extras}${button('Steps')}`);
    await waitFor('failed package step survives restart', () =>
      find(`${extras}//ol/li[.//span[@aria-label='failed'] and .//span[text()='extras'] and .//span[text()='Furnish/Package'] and .//strong[text()='failed']]`));
    const hyperfineCalls = () => log('pkexec.log').split('\n').filter(line => line.includes('pacman -S') && line.endsWith('hyperfine')).length;
    const before = hyperfineCalls();
    await click(`${group}${button('Retry failed')}`);
    await waitFor('same Update all group completes', async () => (await text(summary)).includes('2 of 2 updated'), 30000);
    if (hyperfineCalls() !== before || (await findAll(group)).length !== 1) throw new Error('retry duplicated an update');
    for (const [app, version] of [['hyperfine', '4.0'], ['extras', '2.0']]) {
      if (log(`pkgs/${app}`).trim() !== version) throw new Error(`${app} did not update to ${version}`);
      if (!optOuts().includes(`fixture/${app}`)) throw new Error(`${app} opted back into automatic updates`);
    }
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
      if (attempts !== 1) throw new Error(`${attempts} attempts after a refused one, expected no retry until the next check: ${log('denied.log')}`);
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
  ['collections: clone a release tag, switch to a branch, and resume the default source', async () => {
    await click(nav('Collections'));
    await click(button('Add collection'));
    const allowed = "//details[summary[normalize-space()='Allowed collection URL patterns']]";
    await click(allowed + '/summary');
    if ((await text(allowed + '//code')) !== '.*/second') throw new Error('wrong collection URL policy');
    const url = "//input[@placeholder='git@github.com:your-org/apps.git']";
    await typeInto(url, 'https://furnish.example.test');
    await waitFor('organization address inherits its offered sources', async () =>
      !(await prop(button('Add collection'), 'disabled')) && !(await findAll("//form/details[summary[normalize-space()='Git update source']]")).length);
    await wd('POST', s(`/element/${await find(url)}/clear`), {});
    await typeInto(url, `${workDir}/second`);
    const addSource = "//form/details[summary[normalize-space()='Git update source']]";
    await click(`${addSource}/summary`);
    await click(`${addSource}//select/option[@value='tag']`);
    await typeInto(`${addSource}//input`, '^release-[0-9]+$');
    const order = "//select[option[@value='newest']]";
    if ((await prop(`${addSource}${order}`, 'value')) !== 'version') throw new Error('tag ordering did not default to highest version');
    await click(`${addSource}${order}/option[@value='newest']`);
    await click(button('Add collection'));
    await waitFor('second collection listed', () => find("//li[.//*[normalize-space()='second']]"), 30000);
    // Its required question comes up right away, as a dropdown with labels.
    await waitFor('question dialog', () => find("//dialog[@open]//select[@data-question='role']"));
    if (!(await text('//dialog[@open]')).includes('What do you work on?')) throw new Error('question text missing');
    await click("//dialog[@open]//select[@data-question='role']/option[normalize-space()='Design']");
    await click("//dialog[@open]//button[normalize-space()='Save answers']");
    await waitFor('answer saved', () => /"second":\s*\{\s*"role":\s*"design"/.test(log('data/settings.json')));
    const row = "//li[.//*[normalize-space()='second']]";
    const tracking = `${row}//details[@data-tracking='second']`;
    const revision = (ref) => {
      const git = spawnSync('git', ['-C', `${workDir}/second`, 'rev-parse', `${ref}^{commit}`], { encoding: 'utf8' });
      if (git.status !== 0) throw new Error(git.stderr);
      return git.stdout.trim();
    };
    const expectSource = (ref, choice) => waitFor(`collection source ${ref}`, async () => {
      if (await prop(`${tracking}//select[1]`, 'disabled')) return false;
      const displayed = (await prop(`${row}/span[contains(concat(' ', normalize-space(@class), ' '), ' rev ')]`, 'textContent')).trim();
      const expected = revision(ref);
      const source = await text(`${tracking}//p[contains(@class,'current')]`);
      if (!expected.startsWith(displayed) || !displayed || !source.includes(choice))
        throw new Error(`expected ${ref} (${expected}) and ${choice}, saw '${displayed}' and '${source}'`);
      return true;
    });
    await click(`${tracking}/summary`);
    await expectSource('release-1', 'Most recent tag');
    const savedOrder = () => JSON.parse(log('data/settings.json')).collections.second.tracking.order ?? 'version';
    if (savedOrder() !== 'newest' || (await prop(`${tracking}${order}`, 'value')) !== 'newest')
      throw new Error('newest tag order did not survive the catalog reload');
    await click(`${tracking}${order}/option[@value='version']`);
    await click(`${tracking}${button('Apply and update')}`);
    await expectSource('release-2', 'Highest version');
    if (savedOrder() !== 'version') throw new Error('highest version order was not saved');
    const input = `${tracking}//input`;
    await wd('POST', s(`/element/${await find(input)}/clear`), {});
    await typeInto(input, '[');
    await click(`${tracking}${button('Apply and update')}`);
    await waitFor('invalid pattern has a visible explanation', async () => (await text(`${tracking}//*[@role='alert']`)).trim().length > 0);
    await expectSource('release-2', '^release-[0-9]+$');
    await click(`${tracking}//select/option[@value='branch']`);
    await typeInto(input, 'stable');
    await click(`${tracking}${button('Apply and update')}`);
    await expectSource('stable', 'stable');
    await click(`${tracking}//select/option[@value='inherit']`);
    await click(`${tracking}${button('Apply and update')}`);
    await expectSource('HEAD', 'Furnish default');
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
  ['Activity retains a failed install and can retry it after a fresh launch', async () => {
    writeFileSync(`${workDir}/fail-once-later`, '');
    await click(`${card('later')}${button('Install')}`);
    await click(nav('Activity'));
    const entry = "//li[contains(@class,'history')][.//button[normalize-space()='Retry later']]";
    await waitFor('failed later install', () => find(entry), 30000);
    await startSession();
    await click(nav('Activity'));
    await waitFor('failed step and collection revision survive', async () => {
      const detail = await text(entry);
      return detail.includes('The package manager could not complete this step') && /[a-f0-9]{7,40}/.test(detail);
    });
    await click(`${entry}${button('Steps')}`);
    await waitFor('failed step after restart', () =>
      find(`${entry}//ol/li[.//span[@aria-label='failed'] and .//span[text()='later'] and .//span[text()='Furnish/Package'] and .//strong[text()='failed']]`));
    await click(`${entry}${button('Retry later')}`);
    await waitFor('retry installed the package', () => existsSync(`${workDir}/pkgs/later`), 30000);
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
    const tracking = `${row}//details[@data-tracking='fixture']`;
    await click(`${tracking}/summary`);
    if (!(await text(tracking)).includes('Organization policy') || !(await text(tracking)).includes('Locked') || (await findAll(`${tracking}//select`)).length)
      throw new Error('policy update source is not shown as locked');
  }],
  ["reports: the organization's setting shows and can't be changed", async () => {
    await click(nav('Settings'));
    const section = "//section[h2[normalize-space()='Reports to your organization']]";
    await waitFor('reports section', () => find(section));
    if (!(await text(section)).includes('Set by your organization')) throw new Error('no hint that the organization sets it');
    if (!(await prop(`${section}//select`, 'disabled'))) throw new Error('locked reporting setting is editable');
    if ((await prop(`${section}//select`, 'value')) !== 'errors') throw new Error('not the policy value');
  }],
  ['Settings opens the data, Nix configuration and generated files folders', async () => {
    const config = workDir + '/Nix configuration';
    const generated = '/var/lib/furnish/apps';
    const data = workDir + '/data';
    mkdirSync(config, { recursive: true });
    writeFileSync(config + '/flake.nix', `{ nixosConfigurations."${inContainer(['hostname'])}" = {}; }`);
    inContainer(['mkdir', '-p', '/etc/nixos'], true);
    inContainer(['touch', '/etc/NIXOS'], true);
    inContainer(['ln', '-s', config + '/flake.nix', '/etc/nixos/flake.nix'], true);
    try {
      await startSession();
      await click(nav('Settings'));
      const open = path => `//button[@aria-label='Open ${path} in the file manager']`;
      await waitFor('generated NixOS files folder', () => find(open(generated)));
      if (!(await prop(open(generated), 'disabled'))) throw new Error('missing generated folder can be opened');
      inContainer(['mkdir', '-p', generated], true);
      await startSession();
      await click(nav('Settings'));
      await waitFor('generated folder can be opened', async () => !(await prop(open(generated), 'disabled')));
      for (const path of [config, generated, data]) {
        await click(open(path));
        await waitFor('file manager opened ' + path, () => log('folders.log').split('\n').includes(path));
      }
      if (log('browser.log').split('\n').includes(generated)) throw new Error('folder sent to browser');
    } finally {
      inContainer(['rm', '-f', '/etc/NIXOS', '/etc/nixos/flake.nix'], true);
      await startSession();
    }
  }],
  ['the Settings page shows the version and opens the open-source licenses', async () => {
    await click(nav('Settings'));
    const info = JSON.parse(inContainer(['/usr/local/bin/furnish-gui', '--build-info']));
    const expected = process.env.FURNISH_TEST_RUNNING_VERSION ?? info.version;
    await waitFor('running release version', async () => (await text("//*[@id='version']")) === `furnish ${expected}`);
    await click("//*[@id='build-details']/summary");
    if (await text("//*[@id='build-target']") !== info.target) throw new Error('About shows the wrong build target');
    if (info.revision && await text("//*[@id='build-revision']") !== info.revision) throw new Error('About shows the wrong source commit');
    if (info.sourceDateEpoch != null && await prop("//*[@id='build-details']//time", 'dateTime') !== new Date(info.sourceDateEpoch * 1000).toISOString()) throw new Error('About shows the wrong source date');
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
    const summary = `${group}/details/summary`;
    await waitFor('partial kit result names the failed member', async () => {
      const result = await text(summary);
      return result.includes('1 of 2 installed') && result.includes("Couldn't install toolbox");
    }, 30000);
    await waitFor('expand kit result', async () => {
      if (!(await prop(`${group}/details`, 'open'))) await click(summary);
      return prop(`${group}/details`, 'open');
    });
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
  ['the Linux launcher follows policy changes and preserves a personal entry', async () => {
    const policyPath = `${workDir}/policy.json`;
    const original = readFileSync(policyPath, 'utf8');
    const policy = JSON.parse(original);
    const oldIcon = log(LAUNCHER).match(/^Icon=(.+)$/m)?.[1];
    if (!oldIcon || !existsSync(oldIcon)) throw new Error('branded icon missing before policy update');
    const personal = '[Desktop Entry]\nType=Application\nName=My launcher\nExec=furnish-gui --hidden\n';
    try {
      policy.branding = { name: 'Updated Apps' };
      writeFileSync(policyPath, JSON.stringify(policy));
      await startSession();
      await waitFor('name-only branding', () => log(LAUNCHER).includes('Name=Updated Apps\n') && log(LAUNCHER).includes('Icon=furnish-gui\n') && !existsSync(oldIcon));

      delete policy.branding;
      writeFileSync(policyPath, JSON.stringify(policy));
      await startSession();
      await waitFor('system launcher restored', () => !existsSync(`${workDir}/${LAUNCHER}`));
      if (!log('system-share/applications/furnish.desktop').includes('Name[de]=Paketverwaltung\n')) throw new Error('package launcher was edited');

      writeFileSync(`${workDir}/${LAUNCHER}`, personal);
      writeFileSync(policyPath, original);
      await startSession();
      await waitFor('personal entry while organization branding is active', async () => (await wd('GET', s('/title'))) === 'Acme Apps');
      if (log(LAUNCHER) !== personal) throw new Error('personal launcher changed');
    } finally {
      writeFileSync(policyPath, original);
    }
  }],
  ['closed-app preflight pauses only affected work and rechecks it after restart', async () => {
    const writer = 'furnish-ui-cache';
    const settings = 'preflight-settings', dependent = 'preflight-dependent', free = 'preflight-free';
    const activity = `//li[contains(@class,'row')][.//button[@aria-label='Open ${settings}']]`;
    const recheck = activity + button('Recheck and continue');
    const file = (name) => `${workDir}/${name}`;
    const preferences = JSON.parse(log('data/settings.json') || '{}');
    preferences.noAutoUpdate = [...new Set([...(preferences.noAutoUpdate ?? []), 'fixture/' + settings, 'fixture/' + dependent, 'fixture/' + free])];
    writeFileSync(file('data/settings.json'), JSON.stringify(preferences));
    settingsWriter(true);
    await startSession();
    await waitFor('only the running writer named on the card', async () => (await text(card(settings))).includes('Close ' + writer + ' to continue.'));
    await click(card(dependent) + button('Install'));
    await click(nav('Activity'));
    await waitFor('prerequisite queued for recheck before any step', () => find(recheck));
    if (existsSync(file('preflight-earlier')) || existsSync(file(dependent))) throw new Error('guarded branch started before close');
    await click(recheck);
    await waitFor('still running on recheck', () => find(recheck));
    await click(nav('Discover'));
    await click(card(free) + button('Install'));
    await waitFor('independent work completed', () => log(free) === 'ready');
    await startSession();
    await click(nav('Activity'));
    await waitFor('restored paused branch', () => find(recheck));
    if (existsSync(file('preflight-earlier')) || existsSync(file(dependent))) throw new Error('startup resumed without confirmation');
    settingsWriter(false);
    await click(recheck);
    await waitFor('settings and dependent completed', () => log('preflight-earlier') === 'ready' && log(dependent) === 'ready');
    if (!JSON.parse(log('preflight.json')).enabled) throw new Error('settings not saved');

    // Update all must retain one request, skip the held app, and complete the independent update.
    writeFileSync(file('preflight.json'), '{"enabled":false,"keep":42}');
    writeFileSync(file('preflight-earlier'), 'old');
    writeFileSync(file(free), 'old');
    writeFileSync(file(dependent), 'old');
    await startSession();
    await click(nav('Updates'));
    await waitFor('both updates verified', async () =>
      (await findAll(card(settings) + button('Update'))).length && (await findAll(card(free) + button('Update'))).length
      && (await findAll(card(dependent) + button('Update'))).length);
    // Start the writer after status was checked to exercise the native operation preflight.
    settingsWriter(true);
    await waitFor('writer running', () => {
      const result = spawnSync('docker', ['exec', container, 'sh', '-c',
        'test "$(readlink "/proc/$(cat "$1/writer.pid")/exe")" = "$1/furnish-ui-cache"', 'sh', workDir]);
      return result.status === 0;
    });
    await click(button('Update all') + '[not(@disabled)]');
    await click(nav('Activity'));
    const group = "//li[@data-kind='updates'][.//li[@data-app='fixture/preflight-settings']]";
    await waitFor('group names the running writer and independent update completes', async () =>
      (await text(group + '/details/summary')).includes(writer) && log(free) === 'ready');
    for (const file of ['preflight-earlier', dependent]) {
      if (log(file) !== 'old') throw new Error(`${file} changed before recheck: ${log(file)}; ${await text(group)}`);
    }
    await startSession();
    await click(nav('Activity'));
    await click(group + '/details/summary');
    settingsWriter(false);
    await click(group + "//li[@data-app='fixture/preflight-settings']" + button('Recheck and continue'));
    await waitFor('same update batch finishes', async () => log('preflight-earlier') === 'ready' && log(dependent) === 'ready' && (await text(group)).includes('Updated preflight-settings'));
    if ((await findAll(group)).length !== 1 || JSON.parse(log('preflight.json')).keep !== 42) throw new Error('lost group or unrelated data');
  }],
];

if (replaceDesktop) steps.push(['a Nix generation switch compares the desktop behind the profile wrapper', async () => {
  const profile = '/run/current-system/sw/bin/furnish-gui';
  const offer = "//*[@data-testid='desktop-restart']";
  const installed = JSON.parse(inContainer(['/fixtures/replacement', '--build-info']));
  for (const build of ['running', 'replacement']) {
    const bin = `/nix/store/fixture-${build}-furnish/bin`;
    inContainer(['install', '-D', `/fixtures/${build}`, `${bin}/.furnish-gui-wrapped`], true);
    const wrapper = `#!/bin/sh\nexec "${bin}/.furnish-gui-wrapped" "$@"\n`;
    inContainer(['sh', '-ec', 'printf %s "$1" > "$2" && chmod 755 "$2"', 'sh', wrapper, `${bin}/furnish-gui`], true);
  }
  inContainer(['mkdir', '-p', '/run/current-system/sw/bin'], true);
  const switchTo = (build) => inContainer(['ln', '-sfn', `/nix/store/fixture-${build}-furnish/bin/furnish-gui`, profile], true);
  const focus = () => wd('POST', s('/execute/sync'), {script: "window.dispatchEvent(new Event('focus'));", args: []});
  switchTo('running');
  await startSession(profile);
  await click(nav('Settings'));
  const running = JSON.parse(inContainer(['/fixtures/running', '--build-info']));
  await waitFor('Nix running build', async () => await text("//*[@id='version']") === `furnish ${running.version}`);
  switchTo('replacement');
  await focus();
  await waitFor('installed Nix generation offered', async () => (await text(offer)).includes(installed.version));
  switchTo('running');
  await focus();
  await waitFor('restart offer withdrawn after restoring the running generation', async () => !(await findAll(offer)).length);
}]);

if (replaceDesktop) steps.push(['restart into the installed desktop only when the queue is idle', async () => {
  const settings = 'preflight-settings';
  const offer = "//*[@data-testid='desktop-restart']";
  const running = JSON.parse(inContainer(['/fixtures/running', '--build-info']));
  const installed = JSON.parse(inContainer(['/fixtures/replacement', '--build-info']));
  if (running.version === installed.version) throw new Error('the test needs two distinct desktop versions');
  writeFileSync(`${workDir}/preflight.json`, '{"enabled":false}');
  settingsWriter(true);
  try {
    await startSession();
    await click(nav('Updates'));
    await waitFor('guarded app update', () => find(card(settings) + button('Update')));
    if ((await findAll(offer)).length) throw new Error('restart offered for the running build');
    inContainer(['sh', '-ec', 'install /fixtures/replacement /usr/local/bin/furnish-next && mv -f /usr/local/bin/furnish-next /usr/local/bin/furnish-gui'], true);
    await wd('POST', s('/execute/sync'), {script: "window.dispatchEvent(new Event('focus'));", args: []});
    await waitFor('installed version offered', async () => (await text(offer)).includes(installed.version));
    await click(card(settings) + button('Update'));
    await click(nav('Activity'));
    const recheck = "//li[.//button[@aria-label='Open preflight-settings']]" + button('Recheck and continue');
    await waitFor('paused update', () => find(recheck));
    if ((await findAll(offer)).length) throw new Error('restart offered while a guarded update was queued');
    settingsWriter(false);
    await click(recheck);
    await waitFor('idle desktop restart after the update', async () =>
      JSON.parse(log('preflight.json')).enabled && (await text(offer)).includes(installed.version));
    writeFileSync(screenshot, Buffer.from(await wd('GET', s('/screenshot')), 'base64'));
    // Clicking can tear down the WebDriver session before its reply is delivered.
    const el = await find(offer + '//button');
    await wd('POST', s(`/element/${el}/click`), {}).catch(() => {});
    await waitFor('relaunched desktop running the replacement executable', () =>
      inContainer(['sh', '-c', 'for exe in /proc/[0-9]*/exe; do if [ "$exe" -ef /usr/local/bin/furnish-gui ]; then echo running; break; fi; done']) === 'running');
  } finally {
    // The fixture may still be alive if an assertion failed before Recheck.
    inContainer(['sh', '-c', 'if [ "$(readlink "/proc/$(cat "$1/writer.pid")/exe")" = "$1/furnish-ui-cache" ]; then kill "$(cat "$1/writer.pid")"; fi', 'sh', workDir]);
  }
}]);

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
