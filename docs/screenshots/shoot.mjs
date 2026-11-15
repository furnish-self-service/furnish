// Takes the website's and docs' screenshots of the real furnish app (see run.sh), one scene per
// app start: `main` (furnish's look), `branded` (an organization's), `dark`.
import { writeFileSync } from 'node:fs';

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
  await sleep(700);
  writeFileSync(`${out}/${name}.png`, Buffer.from(await wd('GET', s('/screenshot')), 'base64'));
  console.log(`  ${name}`);
}
/** For phones: also save just the element that matters as <name>-mobile.png (a whole window
 *  shrunk to a phone's width has unreadable text). */
async function crop(name, xpath) {
  // The app's narrowest window (minWidth in tauri.conf.json): fewer columns, narrower crops.
  await wd('POST', s('/window/rect'), { width: 720, height: 790 });
  await sleep(800);
  const id = await find(xpath);
  await wd('POST', s('/execute/sync'), { script: 'arguments[0].scrollIntoView({block: "nearest"})', args: [{ [ELEMENT]: id }] });
  await sleep(400);
  // A window screenshot cut to the element (run.sh crops it): WebKit's element screenshots
  // come out black for elements in scrolling containers.
  const r = await wd('POST', s('/execute/sync'), {
    script: 'const b = arguments[0].getBoundingClientRect(), k = devicePixelRatio; return [b.width, b.height, b.left, b.top].map((v) => Math.round(v * k));',
    args: [{ [ELEMENT]: id }],
  });
  writeFileSync(`${out}/${name}-mobile.png`, Buffer.from(await wd('GET', s('/screenshot')), 'base64'));
  writeFileSync(`${out}/${name}-mobile.crop`, `${r[0]}x${r[1]}+${r[2]}+${r[3]}`);
  console.log(`  ${name}-mobile`);
  await wd('POST', s('/window/rect'), { width: 1180, height: 760 });
  await sleep(800);
}

/** Wait until every app's status is known (no "Checking…" left). */
const settled = () => waitFor('statuses', async () => !(await findAll("//*[normalize-space()='Checking…']")).length, 90000);

const scenes = {
  async main() {
    // First launch: the collection's required question.
    await waitFor('question', () => find("//dialog[@open]//select[@data-question='role']"), 60000);
    await shot('questions');
    await crop('questions', '//dialog[@open]');
    await click("//dialog[@open]//select[@data-question='role']/option[normalize-space()='Design and media']");
    await click("//dialog[@open]//button[normalize-space()='Save answers']");
    await waitFor('recommendations', () => find("//div[contains(@class,'recommended')]//h3[normalize-space()='gimp']"));
    await settled();
    await js('window.scrollTo(0, 0)');
    await shot('discover');
    await crop('discover', "//div[contains(@class,'recommended')]");
    await js('window.scrollTo(0, 0)');

    // An app's page with the preview of its steps.
    await click(`${card('gimp')}//button[@aria-label='Open gimp']`);
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
    await waitFor('installed', () => find("//*[contains(normalize-space(), 'Installed gimp in')]"), 60000);
    await click(nav('Discover'));

    // A kit.
    await click("//button[@aria-label='Open kit design-studio']");
    await shot('kit');
    await crop('kit', "//ul[contains(@class,'items')]");
    await click(nav('Discover'));

    // Several installs at once: the queue on the Activity page.
    for (const app of ['inkscape', 'krita', 'kdenlive']) await click(`${card(app)}//button[normalize-space()='Install']`);
    await click(nav('Activity'));
    await waitFor('queue', async () => (await findAll("//ol[contains(@class,'queue')]/li")).length >= 1);
    await shot('activity');
    await crop('activity', '//main/section');
    await click(nav('Discover'));
    for (const app of ['inkscape', 'krita', 'kdenlive']) {
      await waitFor(`${app} installed`, () => find(`${card(app)}//*[normalize-space()='Installed']`), 90000);
    }

    await click(nav('Updates'));
    await waitFor('update listed', () => find(card('libreoffice')));
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
  async dark() {
    await settled();
    await shot('discover-dark');
    await crop('discover-dark', "//div[contains(@class,'recommended')]");
    await js('window.scrollTo(0, 0)');
    await click(`${card('krita')}//button[@aria-label='Open krita']`);
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
} finally {
  await wd('DELETE', s('')).catch(() => {});
}
