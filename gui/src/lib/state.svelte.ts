// App-wide state and the actions that change it. Components read `store` and call these.
import * as api from './api';
import type { AppInfo, CollectionInfo, KitInfo, LogLine, Outcome, Settings, Status, Step } from './api';
import { applyBranding, type Branding } from './brand';
import { setLanguage, t, formatDate, lang } from './i18n.svelte';
import { serverLooks } from './schedule';
import { contentLanguage, lookup, matches, searchTexts, valueKey } from './content';

export type View =
  | { page: 'discover' }
  | { page: 'installed' }
  | { page: 'updates' }
  | { page: 'category'; name: string }
  | { page: 'collections' }
  | { page: 'settings' }
  | { page: 'activity' }
  | { page: 'kit'; key: string; back: View }
  | { page: 'app'; key: string; back: View };

export type Busy = 'install' | 'uninstall';

/** A step of a run: the planned step plus what happened to it so far. */
export interface RunStep {
  step: Step;
  state: 'pending' | 'running' | 'done' | 'failed' | 'skipped' | 'cancelled';
  startedAt?: number;
  durationMs?: number;
  outcome?: Outcome;
}

export interface Run {
  kind: 'install' | 'update' | 'uninstall' | 'reinstall';
  startedAt: number;
  finishedAt?: number;
  /** Abort requested: 'abort' stops after the current step, 'force' interrupts it. */
  stopping?: 'abort' | 'force';
  cancelled?: boolean;
  steps: RunStep[];
}

export interface Queued {
  key: string;
  action: Busy | 'reinstall' | 'installPrevious' | 'installCurrent';
  /** Started by automatic updates, not by the user. */
  auto?: boolean;
}

export interface Toast {
  id: number;
  kind: 'success' | 'error' | 'info';
  title: string;
  detail?: string;
}

export const store = $state({
  /** The organization's branding; furnish's own until loaded. */
  brand: { name: 'furnish', logo: null, accent: null, accentDark: null, light: {}, dark: {} } as Branding,
  apps: [] as AppInfo[],
  kits: [] as KitInfo[],
  /** Missing key: status not known yet. 'error': the check failed. */
  status: {} as Record<string, Status | 'error'>,
  /** For blocked apps: what they need, e.g. "Homebrew (brew)". */
  reasons: {} as Record<string, string>,
  /** Apps furnish can't uninstall: a resource can't remove what it set up and the app has no removal steps. */
  unremovable: {} as Record<string, true>,
  /** Apps the NixOS or Home Manager configuration installs: shown as installed, left to it. */
  declared: {} as Record<string, api.Declared>,
  busy: {} as Record<string, Busy>,
  collections: [] as CollectionInfo[],
  loading: true,
  view: { page: 'discover' } as View,
  query: '',
  toasts: [] as Toast[],
  /** Activity log of the last install/update/uninstall per app (this session only). */
  logs: {} as Record<string, LogLine[]>,
  /** Step progress of the current or last install/update/uninstall per app (this session only). */
  runs: {} as Record<string, Run>,
  /** Operations waiting for the running one to finish, in order. */
  queue: [] as Queued[],
  /** Installs that failed in this session, and the earlier version that may work. */
  failed: {} as Record<string, { error: string; previous: api.Version | null }>,
  settings: null as Settings | null,
  /** Settings the admin policy locks. */
  locked: [] as string[],
  /** Values from the policy, a furnish address or settings.json that were ignored, and why. */
  warnings: [] as string[],
  /** From the admin policy: whether users may add collections. */
  usersCanAddCollections: true,
  /** App whose optional dependencies the install dialog asks about. */
  choosing: null as string | null,
  /** The dialog with the collections' unanswered required options is open. */
  asking: false,
  /** The organization's furnish addresses and whether the user is signed in to each. */
  endpoints: [] as api.Endpoint[],
  /** While signing in: the sign-in page, in case the browser didn't open. */
  signInUrl: null as string | null,
  /** The furnish address the sign-in in progress is for. */
  signInOrigin: null as string | null,
  /** A furnish:// link waiting for the user's yes: what it would add, or why it can't. */
  link: null as { preview: api.LinkPreview | null; error: string | null } | null,
  /** Background collection updates: when the last one ran and how it went. */
  autoUpdate: {
    running: false,
    lastAt: null as number | null,
    error: null as string | null,
    /** Why automatic app updates are waiting (computer busy), if they are. */
    postponed: null as string | null,
  },
});

api.onProgress((p) => {
  const run = store.runs[p.key];
  if (!run) return;
  if (p.event === 'planned') {
    run.steps = p.steps.map((step) => ({ step, state: 'pending' }));
  } else if (p.event === 'started') {
    Object.assign(run.steps[p.index], { state: 'running', startedAt: p.atMs });
  } else {
    const s = run.steps[p.index];
    s.state = p.outcome.cancelled ? 'cancelled' : p.outcome.error ? 'failed' : p.outcome.action === 'skipped' ? 'skipped' : 'done';
    s.startedAt ??= p.atMs - p.durationMs;
    s.durationMs = p.durationMs;
    s.outcome = p.outcome;
  }
});

function startRun(key: string, kind: Run['kind']) {
  store.logs[key] = [];
  store.runs[key] = { kind, startedAt: Date.now(), steps: [] };
}
function endRun(key: string, cancelled = false) {
  const run = store.runs[key];
  if (run) Object.assign(run, { finishedAt: Date.now(), cancelled });
}

/** Abort the running operation of an app: first after its current step, then (force) right away. */
export async function stop(key: string, how: 'abort' | 'force') {
  const run = store.runs[key];
  if (run) run.stopping = how;
  await (how === 'force' ? api.forceStop() : api.abort());
}

// Operations run one at a time, so log lines belong to whichever app is busy.
// Lines from background status checks are not kept.
api.onLog((line) => {
  const key = Object.keys(store.busy)[0];
  if (!key) return;
  const lines = (store.logs[key] ??= []);
  lines.push(line);
  if (lines.length > 2000) lines.splice(0, lines.length - 2000);
});

let nextToast = 1;
export function toast(kind: Toast['kind'], title: string, detail?: string) {
  const id = nextToast++;
  store.toasts.push({ id, kind, title, detail });
  // Errors stay until dismissed: their details may be needed.
  if (kind !== 'error') setTimeout(() => dismiss(id), 4000);
}
export function dismiss(id: number) {
  store.toasts = store.toasts.filter((t) => t.id !== id);
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export async function loadAll() {
  store.loading = true;
  // Loaded independently: a problem with one must not hide the other.
  const [apps, collections, kits] = await Promise.allSettled([api.listApps(), api.listCollections(), api.listKits()]);
  if (apps.status === 'fulfilled') store.apps = apps.value;
  else toast('error', t('toast.loadApps'), message(apps.reason));
  if (kits.status === 'fulfilled') store.kits = kits.value;
  else toast('error', t('toast.loadKits'), message(kits.reason));
  if (collections.status === 'fulfilled') store.collections = collections.value;
  else toast('error', t('toast.loadCollections'), message(collections.reason));
  askQuestions(); // on launch, and when an update brings a new question
  store.loading = false;
  store.status = {};
  // The engine runs one check at a time anyway; sequential keeps the UI updating card by card.
  for (const app of store.apps) await checkStatus(app.key);
  // The daily inventory report, if the organization takes reports and asks for everything.
  if (store.settings?.reporting === 'all') {
    api.reportInventory(Object.entries(store.status).map(([key, status]) => [key, String(status)])).catch(() => {});
  }
}

export async function checkStatus(key: string) {
  try {
    const check = await api.appStatus(key);
    store.status[key] = check.status;
    if (check.reason) store.reasons[key] = check.reason;
    else delete store.reasons[key];
    if (check.removable) delete store.unremovable[key];
    else store.unremovable[key] = true;
    if (check.declared) store.declared[key] = check.declared;
    else delete store.declared[key];
  } catch {
    store.status[key] = 'error';
  }
}


/** Run one queued operation; resolves to whether it succeeded. */
async function runOperation(app: AppInfo, action: Queued['action'], auto = false): Promise<boolean> {
  const trigger: api.Trigger = autoInstalling.has(app.key) ? 'autoInstall' : auto ? 'update' : 'user';
  const installs = action !== 'uninstall' && action !== 'reinstall';
  const kind: Run['kind'] = installs ? (store.status[app.key] === 'needsUpdate' ? 'update' : 'install') : action;
  startRun(app.key, kind);
  store.busy[app.key] = action === 'uninstall' ? 'uninstall' : 'install';
  delete store.failed[app.key];
  let cancelled = false;
  let ok = false;
  try {
    const fellBack = await api[action](app.key, trigger);
    ok = true;
    if (fellBack)
      toast('info', t('toast.fellBack', { app: appName(app), date: formatDate(fellBack.date) }), t('toast.fellBackDetail', { error: fellBack.error }));
    else toast('success', t(`run.done.${kind}`, { app: appName(app) }));
    if (autoInstalling.delete(app.key)) await api.autoInstallDone(app.key);
  } catch (e) {
    cancelled = message(e) === api.CANCELLED;
    if (cancelled) toast('info', t(`run.cancelled.${kind}`, { app: appName(app) }));
    else {
      toast('error', t(`run.failed.${kind}`, { app: appName(app) }), message(e));
      // An earlier version may work (fallback "ask"); none is offered for one furnish fell back to.
      const offer = action === 'install' && !app.held;
      store.failed[app.key] = { error: message(e), previous: offer ? await api.previousVersion(app.key).catch(() => null) : null };
    }
  } finally {
    endRun(app.key, cancelled);
    delete store.busy[app.key];
    // Which options the app is installed with changed (they decide whether a reinstall is offered).
    if (ok) await reloadApps();
    await checkStatus(app.key);
  }
  return ok;
}

/** Runs a collection change, then reloads the catalog. Returns true on success. */
export async function collectionAction(work: () => Promise<string>, failure: string) {
  try {
    toast('success', await work());
  } catch (e) {
    toast('error', failure, message(e));
    return false;
  }
  await loadAll();
  return true;
}

export function visibleApps(view: View, query: string): AppInfo[] {
  const languages = wanted();
  return store.apps.filter((app) => {
    const status = store.status[app.key];
    if (status === 'unavailable') return false;
    if (view.page === 'installed' && status !== 'installed' && status !== 'needsUpdate') return false;
    if (view.page === 'updates' && status !== 'needsUpdate') return false;
    if (view.page === 'category' && !app.categories.includes(view.name)) return false;
    if (!query.trim()) return true;
    return matches(query, searchTexts('apps', app, textsOf(app.collection), languages));
  });
}

/**
 * Pull all collections. Returns the names that changed; reloads the catalog if any did.
 * Automatic runs stay quiet on failure (e.g. offline): the Settings page shows the last error.
 */
export async function updateCollections(): Promise<string[]> {
  const changed: string[] = [];
  // The organizations' descriptions first: their settings, branding and collections may have changed.
  const errors = await api.refreshEndpoints().catch((e) => [message(e)]);
  await reloadSettings();
  for (const c of store.collections) {
    try {
      const r = await api.updateCollection(c.name);
      if (r.old !== r.new) changed.push(c.name);
    } catch (e) {
      errors.push(`${c.name}: ${message(e)}`);
    }
  }
  if (changed.length) await loadAll();
  // A collection whose update failed (e.g. a commit its checks refuse) shows its update as available.
  else store.collections = await api.listCollections().catch(() => store.collections);
  if (errors.length) throw new Error(errors.join('\n'));
  return changed;
}

const declinedUpdates = new Set<string>();

let retry: ReturnType<typeof setTimeout> | undefined;

/** Queue updates of installed apps that have one, unless turned off for the app. With
 *  load-aware updates on, wait while the computer is busy and ask again a little later. */
async function updateApps() {
  clearTimeout(retry);
  const due = () =>
    store.apps.filter(
      (a) =>
        store.status[a.key] === 'needsUpdate' &&
        !store.settings?.noAutoUpdate.includes(a.key) &&
        !declinedUpdates.has(a.key) &&
        !isPending(a.key),
    );
  if (!due().length) {
    store.autoUpdate.postponed = null;
    return;
  }
  if (store.settings?.loadAwareUpdates) {
    const load = await api.systemLoad().catch(() => null);
    if (load?.busy) {
      store.autoUpdate.postponed = load.busy;
      retry = setTimeout(updateApps, load.retryAfterSeconds * 1000);
      return;
    }
  }
  store.autoUpdate.postponed = null;
  const apps = due();
  for (const app of apps) enqueue(app.key, 'install', true);
  if (apps.length) toast('info', t('toast.autoUpdating', { apps: apps.map(appName).join(', '), n: apps.length }));
}

/** Pull collections, look for app updates (package managers learn about them on their own),
 *  and install those that are due. Runs on launch, on the interval and from "Check for updates". */
export async function checkForUpdates() {
  if (store.autoUpdate.running) return;
  store.autoUpdate.running = true;
  try {
    const changed = await updateCollections();
    store.autoUpdate.error = null;
    if (changed.length) toast('info', t('toast.updated', { names: changed.join(', ') }));
  } catch (e) {
    store.autoUpdate.error = message(e);
  } finally {
    store.autoUpdate.running = false;
    store.autoUpdate.lastAt = Date.now();
  }
  for (const app of store.apps) {
    const s = store.status[app.key];
    if ((s === 'installed' || s === 'needsUpdate') && !isPending(app.key)) await checkStatus(app.key);
  }
  await updateApps();
}

/** Automatic updates for one app (on by default). */
export function setAutoUpdate(key: string, on: boolean) {
  const off = (store.settings?.noAutoUpdate ?? []).filter((k) => k !== key);
  changeSettings({ noAutoUpdate: on ? off : [...off, key] });
}

let timer: ReturnType<typeof setInterval> | undefined;
let lookTimer: ReturnType<typeof setInterval> | undefined;
/** When to only ask the collections' servers whether they have something new (see schedule.ts). */
const looks = () => serverLooks(store.settings?.updateIntervalMinutes ?? 0, store.settings?.updateOnLaunch ?? false);
/** (Re)start the update interval from the current settings. */
function schedule() {
  clearInterval(timer);
  clearInterval(lookTimer);
  const minutes = store.settings?.updateIntervalMinutes ?? 0;
  if (minutes > 0) timer = setInterval(checkForUpdates, minutes * 60_000);
  const every = looks().everyMinutes;
  if (every) lookTimer = setInterval(lookForCollectionUpdates, every * 60_000);
}

/** Which collections have an update that isn't applied yet: asks their servers, then reloads the
 *  list. Offline: the marks stay as they were, no error. */
export async function lookForCollectionUpdates() {
  try {
    await api.checkCollectionServers();
    store.collections = await api.listCollections();
  } catch {
    // offline or a server that doesn't answer: nothing to show
  }
}

/** Collections with an update that isn't applied yet. */
export const collectionUpdates = () => store.collections.filter((c) => c.updateAvailable).length;

/** Load branding and settings (at start, and after the organizations' descriptions changed). */
async function reloadSettings() {
  try {
    store.brand = await api.getBranding();
    applyBranding(store.brand);
  } catch (e) {
    toast('error', t('toast.branding'), message(e));
  }
  try {
    const effective = await api.getSettings();
    store.settings = effective.settings;
    store.locked = effective.locked;
    store.warnings = effective.warnings ?? [];
    store.usersCanAddCollections = effective.usersCanAddCollections;
    setLanguage(effective.settings.language);
  } catch (e) {
    toast('error', t('toast.settings'), message(e));
  }
  schedule();
}

/** App start: load settings and the catalog, then update collections if enabled. */
export async function start() {
  await reloadSettings();
  // furnish:// links: the one furnish started with, and those arriving while it runs.
  api.onLink(openLink);
  api.pendingLink().then((url) => url && openLink(url)).catch(() => {});
  // Collections the organization provides, for this user too. A missing sign-in shows as a banner.
  for (const problem of await api.ensurePresets().catch((e) => [message(e)])) {
    if (!problem.includes('sign in to')) toast('error', t('toast.setupCollection'), problem);
  }
  await refreshEndpoints();
  await loadAll();
  await autoInstall();
  if (store.settings?.updateOnLaunch) await checkForUpdates();
  else {
    updateApps();
    if (looks().atStart) lookForCollectionUpdates();
  }
}

// --- Tray

/** The tray menu's texts and what furnish is doing, in the app's language. */
export function trayTexts(): api.TrayTexts {
  const brand = store.brand.name;
  const busy = Object.keys(store.busy)[0];
  const updates = store.apps.filter((a) => store.status[a.key] === 'needsUpdate').length;
  const busyApp = busy ? appByKey(busy) : undefined;
  const app = busy && (busyApp ? appName(busyApp) : busy);
  const status = app
    ? t(store.busy[busy] === 'uninstall' ? 'tray.uninstalling' : 'tray.installing', { app })
    : store.autoUpdate.running
      ? t('tray.checking')
      : updates
        ? t('tray.updates', { n: updates })
        : '';
  return {
    open: t('tray.open', { brand }),
    check: t('tray.check'),
    settings: t('tray.settings'),
    quit: t('tray.quit', { brand }),
    status,
    tooltip: status ? t('tray.tooltip', { brand, status }) : brand,
  };
}

let sentTray = '';
/** Send the tray's texts when they changed. */
export function syncTray(texts: api.TrayTexts) {
  const json = JSON.stringify(texts);
  if (json === sentTray) return;
  sentTray = json;
  api.setTray(texts).catch(() => {});
}

// The first time the window closes to the tray, say so when it opens again.
const HINT = 'furnish.trayHintShown';
let hintDue = false;
const hintShown = () => {
  try {
    return localStorage.getItem(HINT) === '1';
  } catch {
    return false;
  }
};
api.onToTray(() => {
  if (!hintShown()) hintDue = true;
});
api.onShown(() => {
  if (!hintDue) return;
  hintDue = false;
  toast('info', t('toast.inTray', { brand: store.brand.name }));
  try {
    localStorage.setItem(HINT, '1');
  } catch {}
});
api.onCheckUpdates(() => checkForUpdates());
api.onOpenSettings(() => (store.view = { page: 'settings' }));

api.onSignIn((started) => {
  store.signInUrl = started.url;
  store.signInOrigin = started.origin;
});

export async function refreshEndpoints() {
  store.endpoints = await api.endpoints().catch(() => []);
}

/** Sign in to an organization's furnish address again, then set up its collections. */
export async function signIn(origin: string) {
  try {
    for (const problem of await api.signIn(origin)) toast('error', t('toast.setupCollection'), problem);
    toast('success', t('toast.signedIn', { name: store.endpoints.find((e) => e.origin === origin)?.name ?? origin }));
  } catch (e) {
    toast('error', t('toast.signInFailed'), message(e));
  } finally {
    store.signInUrl = null;
  }
  await refreshEndpoints();
  await loadAll();
}

/** A furnish://add?endpoint=… link: show what it would add and ask before adding anything (a link
 *  can come from anyone). Only furnish addresses, never git URLs or paths. */
let linkSeq = 0;
function openLink(url: string) {
  const seq = ++linkSeq; // a newer link replaces this one
  store.link = { preview: null, error: null };
  api
    .linkPreview(url)
    .then((preview) => {
      if (seq === linkSeq && store.link) store.link = { preview, error: null };
    })
    .catch((e) => {
      if (seq === linkSeq && store.link) store.link = { preview: null, error: message(e) };
    });
}

/** Apps the organization installs on first launch (e.g. its VPN client), once each: an app the
 *  user uninstalls later stays uninstalled. */
async function autoInstall() {
  for (const key of await api.autoInstallPending().catch(() => [])) {
    const app = store.apps.find((a) => a.key === key);
    if (!app) continue; // its collection may arrive later (e.g. after signing in)
    if (isInstalled(key)) {
      await api.autoInstallDone(key);
    } else {
      install(app);
      autoInstalling.add(key);
    }
  }
}
const autoInstalling = new Set<string>();

export const isLocked = (key: keyof Settings) => store.locked.includes(key);

export async function changeSettings(change: Partial<Settings>) {
  if (!store.settings) return;
  try {
    for (const [key, value] of Object.entries(change)) await api.setSetting(key as keyof Settings, value);
    store.settings = { ...store.settings, ...change };
    if (change.language) setLanguage(change.language);
    schedule();
    if ('loadAwareUpdates' in change) await updateApps(); // postponed updates may go ahead now
  } catch (e) {
    toast('error', t('toast.saveSettings'), message(e));
  }
}

/** Dependencies installed with an app unless the user changes the choice. */
export const defaultDependencies = (app: AppInfo) => app.dependencies.filter((d) => d.selected).map((d) => d.key);

/** Install or update an app: right away, or after what is already running and queued.
 *  Its chosen dependencies (default: required + preselected) that are missing are queued first. */
export function install(app: AppInfo, chosen = defaultDependencies(app), seen = new Set<string>()) {
  if (seen.has(app.key)) return; // a dependency cycle: the run itself will report it
  seen.add(app.key);
  for (const key of chosen) {
    const dep = store.apps.find((a) => a.key === key);
    const status = store.status[key];
    if (dep && status !== 'installed' && status !== 'needsUpdate' && status !== 'unavailable') {
      install(dep, defaultDependencies(dep), seen);
    }
  }
  enqueue(app.key, 'install');
}

/** Reinstall an app so its current options apply: uninstalls with the options it was installed
 *  with, then installs with the current ones. Dependencies the new options need are queued first. */
export function reinstall(app: AppInfo) {
  for (const key of defaultDependencies(app)) {
    const dep = store.apps.find((a) => a.key === key);
    if (dep && !isInstalled(key) && store.status[key] !== 'unavailable') install(dep);
  }
  enqueue(app.key, 'reinstall');
}

/** Set an option for a collection or an app (`collection/app`); `null` goes back to the inherited value. */
export async function setOption(target: string, name: string, value: unknown) {
  try {
    await api.setOption(target, name, value);
  } catch (e) {
    toast('error', t('toast.changeOption', { name }), message(e));
  }
  await reloadApps();
}

/** Collections with required options nobody answered yet. */
export const unansweredCollections = () => store.collections.filter((c) => c.options.some((o) => o.missing));

let postponed = false;
/** Ask the collections' open questions, if there are any: on launch and when a collection is
 *  added (`force`). After "Later", not again this session unless forced. */
export function askQuestions(force = false) {
  if (force) postponed = false;
  if (!postponed && unansweredCollections().length) store.asking = true;
}
export function postponeQuestions() {
  postponed = true;
  store.asking = false;
}

/** Store answers to the collections' questions; recommendations and installs follow them. */
export async function answerQuestions(answers: { collection: string; name: string; value: unknown }[]) {
  for (const a of answers) {
    try {
      await api.setOption(a.collection, a.name, a.value);
    } catch (e) {
      toast('error', t('toast.saveAnswer', { collection: a.collection }), message(e));
    }
  }
  await reloadApps();
}

/** Reload apps and collections (their options, dependencies) without rechecking every status. */
async function reloadApps() {
  try {
    [store.apps, store.collections] = await Promise.all([api.listApps(), api.listCollections()]);
  } catch (e) {
    toast('error', t('toast.loadApps'), message(e));
  }
}

/** Apps waiting for a missing tool may be installable now (e.g. Homebrew was just installed). */
async function recheckBlocked() {
  for (const key of Object.keys(store.status)) {
    if (store.status[key] === 'blocked') await checkStatus(key);
  }
}

/** Install the earlier version furnish offers after a failed install. */
export function installPrevious(app: AppInfo) {
  enqueue(app.key, 'installPrevious');
}

/** For an app held at an earlier version: try the collection's current version again. */
export function installCurrent(app: AppInfo) {
  enqueue(app.key, 'installCurrent');
}

/** Uninstall an app: right away, or after what is already running and queued. */
export function uninstall(app: AppInfo) {
  if (store.unremovable[app.key]) return; // furnish would refuse; the app page says why
  enqueue(app.key, 'uninstall');
}

function enqueue(key: string, action: Queued['action'], auto = false) {
  if (store.busy[key] || store.queue.some((q) => q.key === key)) return;
  store.queue.push({ key, action, auto });
  pump();
}

export function dequeue(key: string) {
  store.queue = store.queue.filter((q) => q.key !== key);
}

export function clearQueue() {
  store.queue = [];
}

/** Start the next queued operation unless one is running. One at a time, in order. */
async function pump() {
  if (Object.keys(store.busy).length) return;
  const next = store.queue.shift();
  if (!next) return;
  const app = store.apps.find((a) => a.key === next.key);
  if (app) {
    const ok = await runOperation(app, next.action, next.auto);
    // Don't retry an automatic update the user cancelled or that failed, until the app restarts.
    if (next.auto && !ok) declinedUpdates.add(next.key);
  }
  await recheckBlocked();
  // A cancelled or failed operation only ends itself; the queue carries on.
  pump();
}

// --- Kits

export const isInstalled = (key: string) => store.status[key] === 'installed' || store.status[key] === 'needsUpdate';
export const isPending = (key: string) => !!store.busy[key] || store.queue.some((q) => q.key === key);
export const appByKey = (key: string) => store.apps.find((a) => a.key === key);

// Collection texts in the user's languages (see content.ts). They read the language and the
// settings, so changing either re-renders them.

/** The languages for collection texts, in order: the app's, then the fallback language. */
export const wanted = () => [
  contentLanguage(store.settings?.language, lang(), navigator.language),
  store.settings?.fallbackLanguage ?? 'en',
];
const textsOf = (collection: string) => store.collections.find((c) => c.name === collection);
const translated = (collection: string, get: Parameters<typeof lookup>[2]) => lookup(textsOf(collection), wanted(), get);

export const appName = (app: Pick<AppInfo, 'id' | 'name' | 'collection'>) =>
  translated(app.collection, (f) => f.apps?.[app.id]?.name) ?? app.name ?? app.id;
export const appDescription = (app: AppInfo) => translated(app.collection, (f) => f.apps?.[app.id]?.description) ?? app.description;
export const kitName = (kit: KitInfo) => translated(kit.collection, (f) => f.kits?.[kit.id]?.name) ?? kit.name ?? kit.id;
export const kitDescription = (kit: KitInfo) => translated(kit.collection, (f) => f.kits?.[kit.id]?.description) ?? kit.description;
/** A category's name: as its collection translates it; without a collection (the navigation,
 *  which groups by key), as the first collection that translates it does. */
export function categoryName(key: string, collection?: string): string {
  const get = (f: import('./content').LocaleFile) => f.categories?.[key];
  if (collection) return translated(collection, get) ?? key;
  for (const c of store.collections) {
    const name = lookup(c, wanted(), get);
    if (name) return name;
  }
  return key;
}
export const optionLabel = (collection: string, o: api.Option) =>
  translated(collection, (f) => f.parameters?.[o.name]?.label) ?? o.label ?? o.name;
export const optionDescription = (collection: string, o: api.Option) =>
  translated(collection, (f) => f.parameters?.[o.name]?.description) ?? o.description;
export const choiceLabel = (collection: string, o: api.Option, value: unknown, own: string | null | undefined) =>
  translated(collection, (f) => f.parameters?.[o.name]?.allowedValues?.[valueKey(value)]) ?? own;

/** A kit counts as installed when all its required items are. */
function kitInstalled(kit: KitInfo) {
  return kit.items.some((i) => isInstalled(i.key)) && kit.items.filter((i) => i.required).every((i) => isInstalled(i.key));
}

/** Other installed kits that contain this app. */
export function otherKitsWith(appKey: string, except: KitInfo) {
  return store.kits.filter((k) => k.key !== except.key && kitInstalled(k) && k.items.some((i) => i.key === appKey));
}

/** Installed items to preselect for uninstalling a kit: those no other installed kit needs, and
 *  that furnish can uninstall. */
export function defaultUninstall(kit: KitInfo) {
  return kit.items.map((i) => i.key).filter((key) => isInstalled(key) && !store.unremovable[key] && !otherKitsWith(key, kit).length);
}

/** Queue installing the chosen items, in kit order. */
export function installKit(kit: KitInfo, keys: string[]) {
  for (const item of kit.items) {
    const app = appByKey(item.key);
    if (app && keys.includes(item.key)) install(app);
  }
}

/** Queue uninstalling the chosen items, in reverse kit order (later items may build on earlier). */
export function uninstallKit(kit: KitInfo, keys: string[]) {
  for (const item of [...kit.items].reverse()) {
    const app = appByKey(item.key);
    if (app && keys.includes(item.key)) uninstall(app);
  }
}

export function visibleKits(view: View, query: string): KitInfo[] {
  if (view.page !== 'discover' && view.page !== 'category') return [];
  const languages = wanted();
  return store.kits.filter((kit) => {
    if (kit.items.every((i) => store.status[i.key] === 'unavailable')) return false;
    if (view.page === 'category' && !kit.categories.includes(view.name)) return false;
    if (!query.trim()) return true;
    return matches(query, searchTexts('kits', kit, textsOf(kit.collection), languages));
  });
}

/** Install from a card: ask about optional dependencies first, if there are any left to choose. */
export function requestInstall(app: AppInfo) {
  const choice = app.dependencies.some(
    (d) => !d.required && !isInstalled(d.key) && store.status[d.key] !== 'unavailable' && !isPending(d.key),
  );
  if (choice && store.status[app.key] !== 'needsUpdate') store.choosing = app.key;
  else install(app);
}
