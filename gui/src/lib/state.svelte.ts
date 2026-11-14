// App-wide state and the actions that change it. Components read `store` and call these.
import * as api from './api';
import type { AppInfo, CollectionInfo, KitInfo, LogLine, Outcome, Settings, Status, Step } from './api';

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
  kind: 'install' | 'update' | 'uninstall';
  startedAt: number;
  finishedAt?: number;
  /** Abort requested: 'abort' stops after the current step, 'force' interrupts it. */
  stopping?: 'abort' | 'force';
  cancelled?: boolean;
  steps: RunStep[];
}

export interface Queued {
  key: string;
  action: Busy;
}

export interface Toast {
  id: number;
  kind: 'success' | 'error' | 'info';
  title: string;
  detail?: string;
}

export const store = $state({
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
  settings: null as Settings | null,
  /** Background collection updates: when the last one ran and how it went. */
  autoUpdate: { running: false, lastAt: null as number | null, error: null as string | null },
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

const kindNoun = { install: 'Installation', update: 'Update', uninstall: 'Uninstall' } as const;

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

async function runInstall(app: AppInfo) {
  const updating = store.status[app.key] === 'needsUpdate';
  startRun(app.key, updating ? 'update' : 'install');
  store.busy[app.key] = 'install';
  let cancelled = false;
  try {
    await api.install(app.key);
    toast('success', `${updating ? 'Updated' : 'Installed'} ${app.id}`);
  } catch (e) {
    cancelled = message(e) === api.CANCELLED;
    if (cancelled) toast('info', `${kindNoun[updating ? 'update' : 'install']} of ${app.id} cancelled`);
    else toast('error', `Couldn't ${updating ? 'update' : 'install'} ${app.id}`, message(e));
  } finally {
    endRun(app.key, cancelled);
    delete store.busy[app.key];
    await checkStatus(app.key);
  }
}

async function runUninstall(app: AppInfo) {
  startRun(app.key, 'uninstall');
  store.busy[app.key] = 'uninstall';
  let cancelled = false;
  try {
    await api.uninstall(app.key);
    toast('success', `Uninstalled ${app.id}`);
  } catch (e) {
    cancelled = message(e) === api.CANCELLED;
    if (cancelled) toast('info', `${kindNoun.uninstall} of ${app.id} cancelled`);
    else toast('error', `Couldn't uninstall ${app.id}`, message(e));
  } finally {
    endRun(app.key, cancelled);
    delete store.busy[app.key];
    await checkStatus(app.key);
  }
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

async function autoUpdate() {
  if (store.autoUpdate.running || Object.keys(store.busy).length) return; // next tick will retry
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
}

let timer: ReturnType<typeof setInterval> | undefined;
/** (Re)start the update interval from the current settings. */
function schedule() {
  clearInterval(timer);
  const minutes = store.settings?.updateIntervalMinutes ?? 0;
  if (minutes > 0) timer = setInterval(autoUpdate, minutes * 60_000);
}

/** App start: load settings and the catalog, then update collections if enabled. */
export async function start() {
  store.settings = await api.getSettings().catch(() => ({ updateOnLaunch: true, updateIntervalMinutes: 30 }));
  schedule();
  await loadAll();
  if (store.settings.updateOnLaunch) await autoUpdate();
}

export async function changeSettings(change: Partial<Settings>) {
  if (!store.settings) return;
  const next = { ...store.settings, ...change };
  try {
    await api.saveSettings(next);
    store.settings = next;
    schedule();
  } catch (e) {
    toast('error', "Couldn't save the settings", message(e));
  }
}

/** Install or update an app: right away, or after what is already running and queued.
 *  Missing prerequisites (`requires`) are queued first. */
export function install(app: AppInfo, seen = new Set<string>()) {
  if (seen.has(app.key)) return; // a prerequisite cycle: the run itself will report it
  seen.add(app.key);
  for (const key of app.requires) {
    const dep = store.apps.find((a) => a.key === key);
    const status = store.status[key];
    if (dep && status !== 'installed' && status !== 'needsUpdate' && status !== 'unavailable') install(dep, seen);
  }
  enqueue(app.key, 'install');
}

/** Apps waiting for a missing tool may be installable now (e.g. Homebrew was just installed). */
async function recheckBlocked() {
  for (const key of Object.keys(store.status)) {
    if (store.status[key] === 'blocked') await checkStatus(key);
  }
}

/** Uninstall an app: right away, or after what is already running and queued. */
export function uninstall(app: AppInfo) {
  enqueue(app.key, 'uninstall');
}

function enqueue(key: string, action: Busy) {
  if (store.busy[key] || store.queue.some((q) => q.key === key)) return;
  store.queue.push({ key, action });
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
  if (app) await (next.action === 'install' ? runInstall(app) : runUninstall(app));
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
