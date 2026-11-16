// Takes the website's and docs' screenshots of the real furnish app (see run.sh), one scene per
// app start: `main` (furnish's look), `branded` (an organization's), `dark`, `endpoint`, `reports`,
// `fallback` (German, collection settings, an earlier app version).
import { writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';

const DRIVER = 'http://127.0.0.1:4444';
const [scene, , out] = process.argv.slice(2);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const ELEMENT = 'element-6066-11e4-a52e-4f735466cecf';
let session;

async function wd(method, path, body) {
  const res = await fetch(`${DRIVER}${path}`, { method, headers: { 'content-type': 'application/json' }, body: body && JSON.stringify(body) });
  const json = await res.json();
  if (!res.ok) throw new Error(`${method} ${path}: ${JSON.stringify(json.value)}`);
  return json.value;
}
const s = (path) => `/session/${session}${path}`;
async function waitFor(what, fn, timeout = 30000) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try {
      const v = await fn();
      if (v) return v;
    } catch (e) {
      last = e;
    }
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${what}${last ? `: ${last.message}` : ''}`);
}
const find = async (xpath) => (await wd('POST', s('/element'), { using: 'xpath', value: xpath }))[ELEMENT];
const findAll = async (xpath) => (await wd('POST', s('/elements'), { using: 'xpath', value: xpath })).map((e) => e[ELEMENT]);
const click = async (xpath) =>
  waitFor(`click on ${xpath}`, async () => {
    const id = await find(xpath);
    await wd('POST', s('/execute/sync'), { script: 'arguments[0].scrollIntoView({block: "center"})', args: [{ [ELEMENT]: id }] });
    await wd('POST', s(`/element/${id}/click`), {});
    return true;
  });
const js = (script) => wd('POST', s('/execute/sync'), { script, args: [] });
const button = (label) => `//button[normalize-space()='${label}']`;
const card = (app) => `//article[.//h3[normalize-space()='${app}']]`;
const nav = (label) => `//nav//button[starts-with(normalize-space(), '${label}')]`;

/** Save the window as <name>.png once the page settled (fonts, images, no running animation). */
async function shot(name) {
  await js('return document.fonts.ready.then(() => true)');
  // The pointer off the content (on the sidebar's Categories heading), so no card shows its hover look.
  if (!(await findAll('//dialog[@open]')).length) {
    const heading = (await findAll('//nav/h2'))[0];
    if (heading) await wd('POST', s(`/element/${heading}/click`), {}).catch((e) => console.error(`  pointer: ${e.message}`));
  }
  await sleep(700);
  writeFileSync(`${out}/${name}.png`, Buffer.from(await wd('GET', s('/screenshot')), 'base64'));
  console.log(`  ${name}`);
}
/** For phones: also save just the element that matters as <name>-mobile.png (a whole window
 *  shrunk to a phone's width has unreadable text). Several XPaths: the area around all of them. */
async function crop(name, ...xpaths) {
  // The app's narrowest window (minWidth in tauri.conf.json): fewer columns, narrower crops.
  await wd('POST', s('/window/rect'), { width: 720, height: 790 });
  await sleep(800);
  const ids = [];
  for (const xpath of xpaths) ids.push({ [ELEMENT]: await find(xpath) });
  await wd('POST', s('/execute/sync'), { script: 'arguments[0].scrollIntoView({block: "nearest"})', args: [ids[0]] });
  await sleep(400);
  // A window screenshot cut to the elements (run.sh crops it): WebKit's element screenshots
  // come out black for elements in scrolling containers.
  const r = await wd('POST', s('/execute/sync'), {
    script: `const bs = [...arguments].map((e) => e.getBoundingClientRect()), k = devicePixelRatio;
      const l = Math.min(...bs.map((b) => b.left)), t = Math.min(...bs.map((b) => b.top));
      const w = Math.max(...bs.map((b) => b.right)) - l, h = Math.max(...bs.map((b) => b.bottom)) - t;
      return [w, h, l, t].map((v) => Math.round(v * k));`,
    args: ids,
  });
  writeFileSync(`${out}/${name}-mobile.png`, Buffer.from(await wd('GET', s('/screenshot')), 'base64'));
  writeFileSync(`${out}/${name}-mobile.crop`, `${r[0]}x${r[1]}+${r[2]}+${r[3]}`);
  console.log(`  ${name}-mobile`);
  await wd('POST', s('/window/rect'), { width: 1180, height: 760 });
  await sleep(800);
}

/** Close the toasts that stay (errors) and wait until the others are gone. */
async function noToasts() {
  for (const id of await findAll("//button[@aria-label='Dismiss']")) await wd('POST', s(`/element/${id}/click`), {}).catch(() => {});
  await waitFor('toasts gone', async () => !(await findAll("//button[@aria-label='Dismiss']")).length, 15000);
}

/** Wait until every app's status is known: cards shown, no "Checking…" left for a second (the
 *  checks start a moment after the cards appear). */
async function settled() {
  let quietSince = 0;
  await waitFor('statuses', async () => {
    const quiet = (await findAll('//article')).length && !(await findAll("//*[normalize-space()='Checking…' or normalize-space()='Wird geprüft…']")).length;
    if (!quiet) quietSince = 0;
    else quietSince ||= Date.now();
    return quiet && Date.now() - quietSince >= 1000;
  }, 90000);
}

const scenes = {
  async main() {
    // First launch: the collection's required question.
    await waitFor('question', () => find("//dialog[@open]//select[@data-question='role']"), 60000);
    await shot('questions');
    await crop('questions', '//dialog[@open]');
    await click("//dialog[@open]//select[@data-question='role']/option[normalize-space()='Office work']");
    await click("//dialog[@open]//button[normalize-space()='Save answers']");
    await waitFor('recommendations', () => find("//div[contains(@class,'recommended')]//h3[normalize-space()='Thunderbird']"));
    await settled();
    await js('window.scrollTo(0, 0)');
    await shot('discover');
    await crop('discover', "//div[contains(@class,'recommended')]");
    await js('window.scrollTo(0, 0)');

    // A kit, before Thunderbird is installed.
    await click("//button[@aria-label='Open kit Office']");
    await shot('kit');
    await crop('kit', "//ul[contains(@class,'items')]");
    await click(nav('Discover'));

    // An app's page with the preview of its steps.
    await click(`${card('Thunderbird')}//button[@aria-label='Open Thunderbird']`);
    await click("//details[summary[contains(., 'Steps')]]/summary");
    await waitFor('preview', async () => (await findAll("//details[@open]//li")).length > 0);
    await shot('app-detail');
    await crop('app-detail', "//section[contains(@class,'detail')]/header");
    // Installing it: the same steps, live.
    await click("//button[contains(@class,'action') and normalize-space()='Install']");
    await waitFor('running step', () => find("//li[contains(@class,'running')]"));
    await sleep(1500);
    await shot('install-progress');
    await crop('install-progress', "//details[@open][summary[contains(., 'Steps')]]");
    await waitFor('installed', () => find("//*[contains(normalize-space(), 'Installed Thunderbird in')]"), 60000);
    await noToasts();
    await click(nav('Discover'));


    // Several installs at once: the queue on the Activity page.
    for (const app of ['GIMP', 'Inkscape', 'VLC']) await click(`${card(app)}//button[normalize-space()='Install']`);
    await click(nav('Activity'));
    await waitFor('queue', async () => (await findAll("//ol[contains(@class,'queue')]/li")).length >= 1);
    await shot('activity');
    await crop('activity', '//main/section');
    await click(nav('Discover'));
    for (const app of ['GIMP', 'Inkscape', 'VLC']) {
      await waitFor(`${app} installed`, () => find(`${card(app)}//*[normalize-space()='Installed']`), 90000);
    }

    await click(nav('Updates'));
    await waitFor('update listed', () => find(card('LibreOffice')));
    await noToasts();
    await shot('updates');
    await crop('updates', "//div[contains(@class,'grid')]");

    await click(nav('Collections'));
    await click("//summary[contains(., 'Options for all example apps')]");
    await shot('collections');
    await crop('collections', "//li[.//details[contains(@class,'copts')]]");
  },
  async branded() {
    await settled();
    await shot('branded');
  },
  async endpoint() {
    await settled();
    await waitFor('sign-in banner', () => find("//div[contains(@class,'banner')]//button[normalize-space()='Sign in']"));
    await shot('sign-in');
    await crop('sign-in', "//div[contains(@class,'banner')]");
    // A private repository: the login fields of the add form.
    await click(nav('Collections'));
    const field = "//input[@placeholder='git@github.com:your-org/apps.git']";
    await wd('POST', s(`/element/${await find(field)}/value`), { text: 'https://git.acme.example/it/apps.git' });
    await click("//details[contains(@class,'login')]/summary");
    await wd('POST', s(`/element/${await find("//details[contains(@class,'login')]//input[not(@type='password')]")}/value`), { text: 'alice' });
    await wd('POST', s(`/element/${await find("//details[contains(@class,'login')]//input[@type='password']")}/value`), { text: 'glpat-example-token' });
    await js('window.scrollTo(0, 0)');
    await shot('private-collection');
    await crop('private-collection', '//form');
  },
  async reports() {
    await settled();
    await click(nav('Collections'));
    const section = "//section[@aria-labelledby='reports-title']";
    await waitFor('reports section', () => find(section));
    await js(`document.evaluate("${section}", document, null, 9, null).singleNodeValue.scrollIntoView({block: "center"})`);
    await shot('reports-setting');
    await crop('reports-setting', section);
  },
  async fallback() {
    await settled();
    // Discover in German, then back to the system's language.
    await click(nav('Collections'));
    await click("//select[@id='language']/option[@value='de']");
    await click(nav('Entdecken'));
    await waitFor('recommendations', () => find("//div[contains(@class,'recommended')]//h3[normalize-space()='Thunderbird']"));
    await settled();
    await js('window.scrollTo(0, 0)');
    await shot('discover-de');
    await crop('discover-de', "//div[contains(@class,'recommended')]");
    await click(nav('Sammlungen'));
    await click("//select[@id='language']/option[@value='system']");
    await waitFor('English', () => find(nav('Discover')));

    // A collection's downloads and earlier versions.
    await click("//summary[normalize-space()='Downloads and earlier versions']");
    const row = "//li[.//summary[normalize-space()='Downloads and earlier versions']]";
    await waitFor('collection settings', () => find(`${row}//ul[contains(@class,'settings')]`));
    await js(`document.evaluate("${row}", document, null, 9, null).singleNodeValue.scrollIntoView({block: "start"})`);
    await shot('collection-settings');
    await crop('collection-settings', row);

    // Installing 7zip fails (the collection's latest change); its earlier version is offered.
    await click(nav('Discover'));
    await click(`${card('7-Zip')}//button[@aria-label='Open 7-Zip']`);
    await click("//button[contains(@class,'action') and normalize-space()='Install']");
    const header = "//section[contains(@class,'detail')]/header";
    const offer = "//div[contains(@class,'differ')][.//button[starts-with(normalize-space(), 'Install the version from')]]";
    await waitFor('earlier version offered', () => find(offer), 60000);
    await noToasts();
    await js('window.scrollTo(0, 0)');
    await shot('fallback');
    await crop('fallback', header, offer);
    // Installing that version holds 7zip there.
    await click(`${offer}//button`);
    const held = "//div[contains(@class,'differ')][.//button[normalize-space()='Try the current version']]";
    await waitFor('held', () => find(held), 60000);
    await waitFor('installed', () => find("//*[normalize-space()='Installed and up to date']"), 60000);
    await noToasts();
    await js('window.scrollTo(0, 0)');
    await shot('held');
    await crop('held', header, held);
  },
  async dark() {
    await settled();
    await shot('discover-dark');
    await crop('discover-dark', "//div[contains(@class,'recommended')]");
    await js('window.scrollTo(0, 0)');
    await click(`${card('Firefox')}//button[@aria-label='Open Firefox']`);
    await shot('app-detail-dark');
    await crop('app-detail-dark', "//section[contains(@class,'detail')]/header");
  },
};

session = (
  await waitFor('tauri-driver', () =>
    wd('POST', '/session', { capabilities: { alwaysMatch: { 'tauri:options': { application: '/usr/local/bin/furnish-gui' } } } }),
  )
).sessionId;
try {
  await wd('POST', s('/window/rect'), { width: 1180, height: 760 });
  console.log(`scene ${scene}`);
  await scenes[scene]();
} catch (e) {
  // What the window showed when the scene failed.
  const file = `${tmpdir()}/furnish-screenshots-failed-${scene}.png`;
  await wd('GET', s('/screenshot')).then((png) => (writeFileSync(file, Buffer.from(png, 'base64')), console.error(`window at the failure: ${file}`))).catch(() => {});
  throw e;
} finally {
  await wd('DELETE', s('')).catch(() => {});
}
