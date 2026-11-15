// App-wide state and the actions that change it. Components read `store` and call these.
import * as api from './api';
import type { AppInfo, CollectionInfo, KitInfo, LogLine, Outcome, Settings, Status, Step } from './api';
import { applyBranding, type Branding } from './brand';

export type View =
  | { page: 'discover' }
  | { page: 'installed' }
  | { page: 'updates' }
  | { page: 'category'; name: string }
  | { page: 'collections' }
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
  /** A furnish:// link's address waiting for the user's yes. */
  linkOrigin: null as string | null,
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
  else toast('error', "Couldn't load the apps", message(apps.reason));
  if (kits.status === 'fulfilled') store.kits = kits.value;
  else toast('error', "Couldn't load the kits", message(kits.reason));
  if (collections.status === 'fulfilled') store.collections = collections.value;
  else toast('error', "Couldn't load the collections", message(collections.reason));
  askQuestions(); // on launch, and when an update brings a new question
  store.loading = false;
  store.status = {};
  // The engine runs one check at a time anyway; sequential keeps the UI updating card by card.
  for (const app of store.apps) await checkStatus(app.key);
}

export async function checkStatus(key: string) {
  try {
    const check = await api.appStatus(key);
    store.status[key] = check.status;
    if (check.reason) store.reasons[key] = check.reason;
    else delete store.reasons[key];
  } catch {
    store.status[key] = 'error';
  }
}

const kindNoun = { install: 'Installation', update: 'Update', uninstall: 'Uninstall', reinstall: 'Reinstall' } as const;
const kindDone = { install: 'Installed', update: 'Updated', uninstall: 'Uninstalled', reinstall: 'Reinstalled' } as const;
const kindVerb = { install: 'install', update: 'update', uninstall: 'uninstall', reinstall: 'reinstall' } as const;

/** Run one queued operation; resolves to whether it succeeded. */
async function runOperation(app: AppInfo, action: Queued['action']): Promise<boolean> {
  const installs = action !== 'uninstall' && action !== 'reinstall';
  const kind: Run['kind'] = installs ? (store.status[app.key] === 'needsUpdate' ? 'update' : 'install') : action;
  startRun(app.key, kind);
  store.busy[app.key] = action === 'uninstall' ? 'uninstall' : 'install';
  delete store.failed[app.key];
  let cancelled = false;
  let ok = false;
  try {
    const fellBack = await api[action](app.key);
    ok = true;
    if (fellBack) toast('info', `Installed the version of ${app.id} from ${fellBack.date}`, `The current version failed: ${fellBack.error}`);
    else toast('success', `${kindDone[kind]} ${app.id}`);
    if (autoInstalling.delete(app.key)) await api.autoInstallDone(app.key);
  } catch (e) {
    cancelled = message(e) === api.CANCELLED;
    if (cancelled) toast('info', `${kindNoun[kind]} of ${app.id} cancelled`);
    else {
      toast('error', `Couldn't ${kindVerb[kind]} ${app.id}`, message(e));
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
  const q = query.trim().toLowerCase();
  return store.apps.filter((app) => {
    const status = store.status[app.key];
    if (status === 'unavailable') return false;
    if (view.page === 'installed' && status !== 'installed' && status !== 'needsUpdate') return false;
    if (view.page === 'updates' && status !== 'needsUpdate') return false;
    if (view.page === 'category' && !app.categories.includes(view.name)) return false;
    if (!q) return true;
    return [app.id, app.description, app.collection, ...app.categories].some((s) => s.toLowerCase().includes(q));
  });
}

/**
 * Pull all collections. Returns the names that changed; reloads the catalog if any did.
 * Automatic runs stay quiet on failure (e.g. offline): the Collections page shows the last error.
 */
export async function updateCollections(): Promise<string[]> {
  const changed: string[] = [];
  const errors: string[] = [];
  for (const c of store.collections) {
    try {
      const r = await api.updateCollection(c.name);
      if (r.old !== r.new) changed.push(c.name);
    } catch (e) {
      errors.push(`${c.name}: ${message(e)}`);
    }
  }
  if (changed.length) await loadAll();
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
  if (apps.length) toast('info', `Updating ${apps.map((a) => a.id).join(', ')} automatically`);
}

/** Pull collections, look for app updates (package managers learn about them on their own),
 *  and install those that are due. Runs on launch, on the interval and from "Check for updates". */
export async function checkForUpdates() {
  if (store.autoUpdate.running) return;
  store.autoUpdate.running = true;
  try {
    const changed = await updateCollections();
    store.autoUpdate.error = null;
    if (changed.length) toast('info', `Updated ${changed.join(', ')}`);
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
/** (Re)start the update interval from the current settings. */
function schedule() {
  clearInterval(timer);
  const minutes = store.settings?.updateIntervalMinutes ?? 0;
  if (minutes > 0) timer = setInterval(checkForUpdates, minutes * 60_000);
}

/** App start: load settings and the catalog, then update collections if enabled. */
export async function start() {
  try {
    store.brand = await api.getBranding();
    applyBranding(store.brand);
  } catch (e) {
    toast('error', "Couldn't read the branding", message(e));
  }
  try {
    const effective = await api.getSettings();
    store.settings = effective.settings;
    store.locked = effective.locked;
    store.usersCanAddCollections = effective.usersCanAddCollections;
  } catch (e) {
    toast('error', "Couldn't read the settings", message(e));
  }
  schedule();
  // furnish:// links: the one furnish started with, and those arriving while it runs.
  api.onLink(openLink);
  api.pendingLink().then((url) => url && openLink(url)).catch(() => {});
  // Collections the organization provides, for this user too. A missing sign-in shows as a banner.
  for (const problem of await api.ensurePresets().catch((e) => [message(e)])) {
    if (!problem.includes('sign in to')) toast('error', "Couldn't set up a collection", problem);
  }
  await refreshEndpoints();
  await loadAll();
  await autoInstall();
  if (store.settings?.updateOnLaunch) await checkForUpdates();
  else updateApps();
}

api.onSignIn((url) => (store.signInUrl = url));

export async function refreshEndpoints() {
  store.endpoints = await api.endpoints().catch(() => []);
}

/** Sign in to an organization's furnish address again, then set up its collections. */
export async function signIn(origin: string) {
  try {
    for (const problem of await api.signIn(origin)) toast('error', "Couldn't set up a collection", problem);
    toast('success', `Signed in to ${store.endpoints.find((e) => e.origin === origin)?.name ?? origin}`);
  } catch (e) {
    toast('error', "Couldn't sign in", message(e));
  } finally {
    store.signInUrl = null;
  }
  await refreshEndpoints();
  await loadAll();
}

/** A furnish://add?endpoint=… link: ask before adding anything (a link can come from anyone). */
function openLink(url: string) {
  const match = url.match(/^furnish:\/\/add\?endpoint=([^&#]+)/);
  if (match) store.linkOrigin = decodeURIComponent(match[1]);
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
    schedule();
    if ('loadAwareUpdates' in change) await updateApps(); // postponed updates may go ahead now
  } catch (e) {
    toast('error', "Couldn't save the settings", message(e));
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
    toast('error', `Couldn't change ${name}`, message(e));
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
      toast('error', `Couldn't save the answer for ${a.collection}`, message(e));
    }
  }
  await reloadApps();
}

/** Reload apps and collections (their options, dependencies) without rechecking every status. */
async function reloadApps() {
  try {
    [store.apps, store.collections] = await Promise.all([api.listApps(), api.listCollections()]);
  } catch (e) {
    toast('error', "Couldn't load the apps", message(e));
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
    const ok = await runOperation(app, next.action);
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

/** A kit counts as installed when all its required items are. */
function kitInstalled(kit: KitInfo) {
  return kit.items.some((i) => isInstalled(i.key)) && kit.items.filter((i) => i.required).every((i) => isInstalled(i.key));
}

/** Other installed kits that contain this app. */
export function otherKitsWith(appKey: string, except: KitInfo) {
  return store.kits.filter((k) => k.key !== except.key && kitInstalled(k) && k.items.some((i) => i.key === appKey));
}

/** Installed items to preselect for uninstalling a kit: those no other installed kit needs. */
export function defaultUninstall(kit: KitInfo) {
  return kit.items.map((i) => i.key).filter((key) => isInstalled(key) && !otherKitsWith(key, kit).length);
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
  const q = query.trim().toLowerCase();
  return store.kits.filter((kit) => {
    if (kit.items.every((i) => store.status[i.key] === 'unavailable')) return false;
    if (view.page === 'category' && !kit.categories.includes(view.name)) return false;
    if (!q) return true;
    return [kit.id, kit.description, kit.collection, ...kit.categories].some((s) => s.toLowerCase().includes(q));
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
