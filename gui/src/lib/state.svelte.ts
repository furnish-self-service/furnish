// App-wide state and the actions that change it. Components read `store` and call these.
import * as api from './api';
import { defaultDependencies, planInstall, releaseDependents, type Queued } from './install-queue';
export { defaultDependencies, type Queued } from './install-queue';
import { completedActivity, historyLimit, interruptedDependents, isKit, kitItemActive, readHistory, recordProgress, trimHistory, writeHistory, type ActivityEntry, type KitActivity, type Run } from './activity-history';
import { StatusCache } from './status-cache';
import type { AppInfo, CollectionInfo, KitInfo, LogLine, Settings, Status } from './api';
import { applyBranding, type Branding } from './brand';
import { setLanguage, t, terror, formatDate, lang } from './i18n.svelte';
import { serverLooks } from './schedule';
import { contentLanguage, lookup, matches, searchTexts, valueKey } from './content';
import { urgent, urgentAction } from './required';

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
  /** Last known result. 'error' means no successful result is available. */
  status: {} as Record<string, Status | 'error'>,
  statusErrors: {} as Record<string, string>,
  checkedAt: {} as Record<string, number>,
  checkingStatus: {} as Record<string, true>,
  /** For blocked apps: what they need, e.g. "Homebrew (brew)". */
  reasons: {} as Record<string, string>,
  /** Apps furnish can't uninstall: a resource can't remove what it set up and the app has no removal steps. */
  unremovable: {} as Record<string, true>,
  /** Apps installed outside furnish (NixOS, Home Manager, another tool): shown as installed, left alone. */
  external: {} as Record<string, api.External>,
  /** Apps whose collection lists required versions that aren't met here (see required.ts). */
  required: {} as Record<string, api.Required>,
  busy: {} as Record<string, Busy>,
  collections: [] as CollectionInfo[],
  loading: true,
  settingUpCollections: true,
  catalogError: false,
  scan: { running: false, complete: false, checked: 0, total: 0 },
  view: { page: 'discover' } as View,
  query: '',
  toasts: [] as Toast[],
  /** Activity log of the last install/update/uninstall per app (this session only). */
  logs: {} as Record<string, LogLine[]>,
  /** Step progress of the current or last install/update/uninstall per app (this session only). */
  runs: {} as Record<string, Run>,
  recent: [] as ActivityEntry[],
  /** Operations waiting for the running one to finish, in order. */
  queue: [] as Queued[],
  /** Installs that failed in this session, and the earlier version that may work. */
  failed: {} as Record<string, { error: string; previous: api.Version | null }>,
  settings: null as Settings | null,
  /** Settings the admin policy locks. */
  locked: [] as string[],
  /** Values from the policy, a furnish address or settings.json that were ignored, and why. */
  warnings: [] as import('./i18n.ts').SettingsWarning[],
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
  /** furnish's own updates: how it was installed, a newer version, its download. */
  selfUpdate: null as api.SelfUpdateStatus | null,
  /** Restart to update was chosen: it happens once nothing is installing. */
  restartWanted: false,
});

api.onProgress((p) => {
  const run = store.runs[p.key];
  if (run && run.finishedAt === undefined) recordProgress(run, p);
});

let historyProfile: string | undefined;
function initializeActivity(dataDir: string) {
  if (historyProfile === dataDir) return;
  // The profile is fixed for the process. A later catalog load can recover from a
  // failed initial data_dir call; keep anything completed in the meantime.
  const current = historyProfile === undefined ? store.recent : [];
  historyProfile = dataDir;
  store.recent = trimHistory([...current, ...readHistory(dataDir)]);
  if (current.length) writeHistory(dataDir, store.recent);
}

function startRun(app: AppInfo, kind: Run['kind'], trigger: api.Trigger) {
  store.logs[app.key] = [];
  store.runs[app.key] = {
    id: crypto.randomUUID(), name: appName(app), kind, trigger, startedAt: Date.now(), steps: [], phaseStart: 0,
  };
  const run = store.runs[app.key];
  if (kind !== 'uninstall') {
    // Version-specific installs can also repair a blocked prerequisite.
    kitItemRetried(app.key);
    for (const kit of store.recent.filter(isKit)) {
      const item = kit.items.find(i => i.key === app.key && i.state === 'queued');
      if (item) {
        item.state = 'running';
        item.runId = run.id;
      }
    }
  }
  return run;
}

function endRun(key: string, run: Run, result: NonNullable<Run['result']>) {
  run.finishedAt = Date.now();
  run.result = result;
  const entry = completedActivity(key, run, store.logs[key]);
  let ownedByKit = false;
  for (const kit of store.recent.filter(isKit)) {
    const item = kit.items.find(i => i.runId === run.id);
    if (!item) continue;
    ownedByKit ||= !!item.ownsJob;
    item.state = result;
    item.attempts = [entry, ...item.attempts].slice(0, historyLimit);
    delete item.runId;
    delete item.waitingFor;
    kit.updatedAt = run.finishedAt;
  }
  if (!ownedByKit) store.recent.unshift(entry);
  saveActivity();
}

function saveActivity() {
  for (const kit of store.recent.filter(isKit)) {
    for (const item of kit.items) {
      if (item.state !== 'queued') continue;
      const job = store.queue.find(q => q.key === item.key);
      if (job) item.waitingFor = [...job.prerequisites];
    }
  }
  const time = (entry: ActivityEntry) => isKit(entry) ? entry.updatedAt : entry.summary.finishedAt;
  store.recent = trimHistory([...store.recent].sort((a, b) => time(b) - time(a)));
  if (historyProfile) writeHistory(historyProfile, store.recent);
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

let catalogLoad = 0;
let catalogListed: Promise<void>;
let latestLoad: Promise<void>;
let reportedCatalog = 0;
let cache: StatusCache | undefined;
const revisions = new Map<string, string>();
// Cached values are for display. Automatic updates and inventory use live checks.
const verifiedKeys = new Set<string>();
const pendingChecks = new Map<string, symbol>();

export function loadAll(): Promise<void> {
  const load = ++catalogLoad;
  if (!store.autoUpdate.running) checkRound++;
  catalogListed = listCatalog(load);
  latestLoad = catalogListed.then(async () => {
    if (load !== catalogLoad) return latestLoad;
    await scanApps(store.apps, load);
    if (load !== catalogLoad) return latestLoad;
    reportInventory();
  });
  return latestLoad;
}

// A daily inventory must include newly provisioned collections and a completed local scan.
function reportInventory() {
  if (store.settingUpCollections || !store.scan.complete || store.catalogError
    || reportedCatalog === catalogLoad || store.settings?.reporting !== 'all') return;
  reportedCatalog = catalogLoad;
  api.reportInventory(Array.from(verifiedKeys, key => [key, store.status[key]])).catch(() => {});
}

async function listCatalog(load: number) {
  store.loading = true;
  store.scan.complete = false;
  // Loaded independently: a problem with one must not hide the other.
  const [apps, collections, kits, dataDir] = await Promise.allSettled([
    api.listApps(), api.listCollections(), api.listKits(), api.dataDir(),
  ]);
  if (load !== catalogLoad) return;
  store.catalogError = apps.status === 'rejected' || collections.status === 'rejected';
  if (apps.status === 'fulfilled') {
    store.apps = apps.value;
    // A blocked queue can be idle when a collection disappears.
    for (const job of store.queue) if (!appByKey(job.key)) dequeue(job.key);
  } else toast('error', t('toast.loadApps'), message(apps.reason));
  if (kits.status === 'fulfilled') store.kits = kits.value;
  else toast('error', t('toast.loadKits'), message(kits.reason));
  if (collections.status === 'fulfilled') store.collections = collections.value;
  else toast('error', t('toast.loadCollections'), message(collections.reason));
  askQuestions(); // on launch, and when an update brings a new question
  if (dataDir.status === 'fulfilled') initializeActivity(dataDir.value);
  cache = dataDir.status === 'fulfilled' ? new StatusCache(dataDir.value) : undefined;
  revisions.clear();
  pendingChecks.clear();
  verifiedKeys.clear();
  store.status = {};
  store.statusErrors = {};
  store.checkedAt = {};
  store.checkingStatus = {};
  store.reasons = {};
  store.unremovable = {};
  store.external = {};
  store.required = {};
  if (apps.status === 'fulfilled' && collections.status === 'fulfilled') {
    const byCollection = new Map(collections.value.map(c => [c.name, c.revision]));
    cache?.prune(new Set(apps.value.map(app => app.key)));
    for (const app of apps.value) {
      const revision = byCollection.get(app.collection);
      if (!revision || revision === 'unknown') continue;
      revisions.set(app.key, revision);
      const saved = cache?.read(app.key, revision);
      if (saved) showCheck(app.key, saved.check, saved.checkedAt);
    }
  }
  store.loading = false;
}

/** The engine checks one app at a time. A newer catalog scan takes over its progress. */
async function scanApps(apps: AppInfo[], load: number) {
  store.scan = { running: true, complete: false, checked: 0, total: apps.length };
  try {
    for (const app of apps) {
      if (load !== catalogLoad) return;
      await checkStatus(app.key);
      if (load === catalogLoad) store.scan.checked++;
    }
    if (load === catalogLoad) store.scan.complete = true;
  } finally {
    if (load === catalogLoad) store.scan.running = false;
  }
}

function showCheck(key: string, check: api.AppCheck, checkedAt: number | null) {
  if (checkedAt !== null) store.checkedAt[key] = checkedAt;
  delete store.statusErrors[key];
  store.status[key] = check.status;
  if (check.reason) store.reasons[key] = check.reason;
  else delete store.reasons[key];
  if (check.removable) delete store.unremovable[key];
  else store.unremovable[key] = true;
  if (check.external) store.external[key] = check.external;
  else delete store.external[key];
  if (check.required) store.required[key] = check.required;
  else delete store.required[key];
}

/** Ask for an app's status. A security update its collection requires is queued right away,
 *  whatever the automatic-update settings say. */
export async function checkStatus(key: string): Promise<boolean> {
  const load = catalogLoad;
  const request = Symbol();
  pendingChecks.set(key, request);
  store.checkingStatus[key] = true;
  const current = () => load === catalogLoad && pendingChecks.get(key) === request;
  try {
    const check = await api.appStatus(key);
    if (!current()) return false;
    const checkedAt = Date.now();
    showCheck(key, check, checkedAt);
    verifiedKeys.add(key);
    // A live check can also observe a prerequisite installed outside Furnish.
    if (check.status === 'installed' && !isPending(key) && store.queue.some(q => q.prerequisites.includes(key))) {
      kitPrerequisiteAvailable(key);
      releaseDependents(store.queue, key);
      pump();
    }
    const revision = revisions.get(key);
    if (revision) cache?.write(key, revision, check, checkedAt);
    installRequiredUpdate(key);
    return true;
  } catch (error) {
    if (current()) {
      store.status[key] ??= 'error';
      store.statusErrors[key] = message(error);
      verifiedKeys.delete(key);
    }
    return false;
  } finally {
    if (pendingChecks.get(key) === request) {
      pendingChecks.delete(key);
      delete store.checkingStatus[key];
    }
  }
}

/** Rounds of status checks: each check for updates (and each reload of the catalog outside one)
 *  is one. A required update is tried once per round, so one that fails or is refused waits for
 *  the next check instead of starting again from the status check after it. */
let checkRound = 0;
const requiredTried = new Map<string, number>();

/** Queue the security update an app's collection requires, if it has one: right away, also with
 *  automatic updates off for the app, after a declined update and while the computer is busy.
 *  It waits for the install that is running (one at a time). */
function installRequiredUpdate(key: string) {
  if (store.settingUpCollections) return;
  const app = appByKey(key);
  if (!app) return;
  const tried = requiredTried.get(key) === checkRound;
  const action = urgentAction(app, store.required[key], { pending: isPending(key), declared: !!store.external[key], tried });
  if (!action) return;
  requiredTried.set(key, checkRound);
  enqueue(key, action, 'update');
  toast('info', t('toast.requiredUpdate', { app: appName(app) }));
}

/** The app has a security update its collection requires. */
export const requiredUpdate = (key: string) => store.status[key] === 'needsUpdate' && urgent(store.required[key]);


/** Run one queued operation; resolves to whether it succeeded. */
async function runOperation(app: AppInfo, action: Queued['action'], trigger: api.Trigger, commit?: string): Promise<boolean> {
  const installs = action !== 'uninstall' && action !== 'reinstall';
  const kind: Run['kind'] = installs ? (store.status[app.key] === 'needsUpdate' ? 'update' : 'install') : action;
  const run = startRun(app, kind, trigger);
  store.busy[app.key] = action === 'uninstall' ? 'uninstall' : 'install';
  delete store.failed[app.key];
  let cancelled = false;
  let ok = false;
  try {
    const fellBack = action === 'installVersion' ? await api.installVersion(app.key, commit ?? '', trigger) : await api[action](app.key, trigger);
    ok = true;
    if (fellBack) {
      run.fallbackDate = fellBack.date;
      toast('info', t('toast.fellBack', { app: appName(app), date: formatDate(fellBack.date) }), t('toast.fellBackDetail', { error: fellBack.error }));
    } else toast('success', t(`run.done.${kind}`, { app: appName(app) }));
    if (autoInstalling.has(app.key)) await api.autoInstallDone(app.key);
  } catch (e) {
    run.error = message(e);
    cancelled = run.error === api.CANCELLED;
    if (cancelled) toast('info', t(`run.cancelled.${kind}`, { app: appName(app) }));
    else {
      toast('error', t(`run.failed.${kind}`, { app: appName(app) }), terror(message(e)));
      // An earlier version may work (fallback "ask"); none is offered for one furnish fell back to.
      const offer = action === 'install' && !app.held;
      store.failed[app.key] = { error: message(e), previous: offer ? await api.previousVersion(app.key).catch(() => null) : null };
    }
  } finally {
    autoInstalling.delete(app.key);
    endRun(app.key, run, cancelled ? 'cancelled' : !ok ? 'failed' : run.fallbackDate ? 'fallback' : 'done');
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
  const apps = store.apps.filter((app) => {
    const status = store.status[app.key];
    if (status === 'unavailable' && store.statusErrors[app.key] === undefined) return false;
    if (view.page === 'installed' && status !== 'installed' && status !== 'needsUpdate') return false;
    if (view.page === 'updates' && status !== 'needsUpdate') return false;
    if (view.page === 'category' && !app.categories.includes(view.name)) return false;
    if (!query.trim()) return true;
    return matches(query, searchTexts('apps', app, textsOf(app.collection), languages));
  });
  // Updates: required security updates first.
  if (view.page === 'updates') apps.sort((a, b) => Number(requiredUpdate(b.key)) - Number(requiredUpdate(a.key)));
  return apps;
}

/**
 * Pull all collections. Returns the names that changed; reloads the catalog if any did.
 * Automatic runs stay quiet on failure (e.g. offline): the Settings page shows the last error.
 */
export async function updateCollections(): Promise<string[]> {
  const changed: string[] = [];
  // The organizations' descriptions first: their settings, branding and collections may have changed.
  const refreshed = await api.refreshEndpoints().catch((e) => ({ problems: [message(e)], added: [] as string[] }));
  const errors = refreshed.problems;
  await reloadSettings();
  for (const c of store.collections) {
    try {
      const r = await api.updateCollection(c.name);
      if (r.old !== r.new) changed.push(c.name);
    } catch (e) {
      errors.push(`${c.name}: ${message(e)}`);
    }
  }
  if (refreshed.added.length) toast('info', t('collections.added', { names: refreshed.added.join(', ') }));
  // Also asks the new collections' questions (unless postponed this session).
  if (changed.length || refreshed.added.length) await loadAll();
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
  if (store.settingUpCollections) return;
  clearTimeout(retry);
  const due = () =>
    store.apps.filter(
      (a) =>
        verifiedKeys.has(a.key) && store.status[a.key] === 'needsUpdate' &&
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
  for (const app of apps) enqueue(app.key, 'install', 'update');
  if (apps.length) toast('info', t('toast.autoUpdating', { apps: apps.map(appName).join(', '), n: apps.length }));
}

/** Pull collections, look for app updates (package managers learn about them on their own),
 *  and install those that are due. Runs on launch, on the interval and from "Check for updates". */
export async function checkForUpdates() {
  if (store.autoUpdate.running) return;
  store.autoUpdate.running = true;
  const catalogAtStart = catalogLoad;
  const catalogWasChecking = !!latestLoad && (store.loading || store.scan.running);
  checkRound++;
  try {
    try {
      const changed = await updateCollections();
      store.autoUpdate.error = null;
      if (changed.length) toast('info', t('toast.updated', { names: changed.join(', ') }));
    } catch (e) {
      store.autoUpdate.error = message(e);
    }
    checkSelfUpdate();
    // A catalog reload already checks every app, including newly added ones.
    if (catalogWasChecking || catalogLoad !== catalogAtStart) {
      await latestLoad;
    } else {
      const due = store.apps.filter(app =>
        !isPending(app.key) && (isInstalled(app.key) || !verifiedKeys.has(app.key)));
      await scanApps(due, catalogAtStart);
      if (catalogLoad !== catalogAtStart) await latestLoad;
    }
    await updateApps();
  } finally {
    store.autoUpdate.lastAt = Date.now();
    store.autoUpdate.running = false;
  }
}

/** Automatic updates for one app (on by default). */
export function setAutoUpdate(key: string, on: boolean) {
  const off = (store.settings?.noAutoUpdate ?? []).filter((k) => k !== key);
  changeSettings({ noAutoUpdate: on ? off : [...off, key] });
}

// --- furnish's own updates

let selfCheckedAt = 0;
/** Ask for a newer furnish (one small request; offline it says nothing). At start, with the
 *  update interval and from Check now; at most once a minute. */
export async function checkSelfUpdate(download = false) {
  if (!download && Date.now() - selfCheckedAt < 60_000) return;
  selfCheckedAt = Date.now();
  store.selfUpdate = await api.checkSelfUpdate(download).catch(() => store.selfUpdate);
  maybeApplySelfUpdate();
}

const idle = () => !Object.keys(store.busy).length && !store.queue.length;

/** Install a downloaded furnish once nothing is installing: when Restart to update was chosen,
 *  or (automatic updates) while the window is closed to the tray. */
export async function maybeApplySelfUpdate() {
  const s = store.selfUpdate;
  if (s?.state !== 'ready' || !idle()) return;
  if (!store.restartWanted && s.mode !== 'auto') return;
  try {
    await api.applySelfUpdate(!store.restartWanted); // on success furnish restarts
  } catch (e) {
    store.restartWanted = false;
    toast('error', t('settings.selfUpdateFailed', { brand: store.brand.name }), message(e));
  }
}

export function restartToUpdate() {
  store.restartWanted = true;
  maybeApplySelfUpdate();
}

api.onSelfUpdate((s) => {
  store.selfUpdate = s;
  maybeApplySelfUpdate();
});
api.onRestartToUpdate(restartToUpdate);
api.onToTray(() => maybeApplySelfUpdate());
// While the window stays closed, try again from time to time (an install may have ended).
setInterval(maybeApplySelfUpdate, 60_000);

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
  api.selfUpdateStatus().then((s) => (store.selfUpdate ??= s)).catch(() => {});
  // At start unless checking is Never and updating at start is off (then no network traffic at all).
  if (store.settings?.updateOnLaunch || (store.settings?.updateIntervalMinutes ?? 0) > 0) checkSelfUpdate();
  // furnish:// links: the one furnish started with, and those arriving while it runs.
  api.onLink(openLink);
  api.pendingLink().then((url) => url && openLink(url)).catch(() => {});
  // Publish the local catalog first. Its live checks can proceed while presets are fetched.
  loadAll();
  await catalogListed;
  try {
    for (const problem of await api.ensurePresets().catch((e) => [message(e)])) {
      if (!problem.includes('sign in to')) toast('error', t('toast.setupCollection'), problem);
    }
    const collections = await api.listCollections();
    // Presets can be added or replaced under the same name. Recheck their catalog only
    // when it changed, so a slow endpoint doesn't restart a scan of the same local apps.
    const changed = collections.length !== store.collections.length || collections.some((c, i) => {
      const previous = store.collections[i];
      return previous.name !== c.name || previous.url !== c.url || previous.revision !== c.revision || previous.managed !== c.managed;
    });
    if (changed || store.catalogError) loadAll();
  } catch (e) {
    toast('error', t('toast.loadCollections'), message(e));
  }
  await Promise.all([refreshEndpoints(), catalogListed]);
  store.settingUpCollections = false;
  for (const key of verifiedKeys) installRequiredUpdate(key);
  reportInventory();
  await latestLoad;
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
    restart: t('tray.restartToUpdate'),
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

/** A first-launch install can enqueue dependencies too. Wait if any selected app's
 *  last check failed, including one whose cached state still says it is installed. */
function verifiedInstall(app: AppInfo, seen = new Set<string>()): boolean {
  if (!verifiedKeys.has(app.key)) return false;
  if (seen.has(app.key) || isInstalled(app.key) || store.status[app.key] === 'unavailable') return true;
  seen.add(app.key);
  return defaultDependencies(app).every(key => {
    const dep = appByKey(key);
    return !dep || verifiedInstall(dep, seen);
  });
}

/** Apps the organization installs on first launch (e.g. its VPN client), once each: an app the
 *  user uninstalls later stays uninstalled. */
async function autoInstall() {
  for (const key of await api.autoInstallPending().catch(() => [])) {
    const app = store.apps.find((a) => a.key === key);
    if (!app || store.status[key] === 'unavailable' || !verifiedInstall(app)) continue;
    if (isInstalled(key)) {
      await api.autoInstallDone(key);
    } else {
      autoInstalling.add(key);
      install(app, defaultDependencies(app), 'autoInstall');
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
    if ('selfUpdate' in change) {
      store.selfUpdate = await api.selfUpdateStatus();
      if (change.selfUpdate !== 'off') checkSelfUpdate(change.selfUpdate === 'auto');
    }
  } catch (e) {
    toast('error', t('toast.saveSettings'), message(e));
  }
}

/** Install the selected dependencies before their dependent, joining work already underway. */
export function install(app: AppInfo, chosen = defaultDependencies(app), trigger: api.Trigger = 'user', onMember?: (key: string, scheduled: boolean) => void) {
  return scheduleInstall(app, chosen, 'install', trigger, onMember);
}

function scheduleInstall(app: AppInfo, chosen: string[], action: 'install' | 'reinstall', trigger: api.Trigger, onMember?: (key: string, scheduled: boolean) => void) {
  const pending = activeRequest ? [activeRequest, ...store.queue] : store.queue;
  const plan = planInstall(app, chosen, store.apps, pending, key => isInstalled(key) || store.status[key] === 'unavailable');
  if (plan.error) {
    toast('error', t(`queue.${plan.error.kind}`, { detail: plan.error.detail }));
    return false;
  }
  if (onMember) {
    const scheduled = new Set(pending.filter(job => job.action !== 'uninstall').map(job => job.key));
    for (const job of plan.installs) scheduled.add(job.key);
    for (const key of plan.members) onMember(key, scheduled.has(key));
  }
  for (const job of plan.installs) {
    if (store.queue.some(waiting => waiting.prerequisites.includes(job.key))) kitItemRetried(job.key);
    store.queue.push({ ...job, trigger, action: job.key === app.key ? action : 'install' });
  }
  pump();
  return true;
}

/** Reinstall using the new options after their selected prerequisites succeed. */
export function reinstall(app: AppInfo) {
  scheduleInstall(app, defaultDependencies(app), 'reinstall', 'user');
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

/** Install a version of the app the person chose (see api.appVersions) and keep it there. */
export function installVersion(app: AppInfo, commit: string) {
  enqueue(app.key, 'installVersion', 'user', commit);
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

let activeRequest = $state<Queued>();
let pumping = false;

function enqueue(key: string, action: Queued['action'], trigger: api.Trigger = 'user', commit?: string) {
  if (isPending(key)) return;
  store.queue.push({ key, action, trigger, commit, prerequisites: [] });
  pump();
}

export function dequeue(key: string) {
  store.queue = store.queue.filter(q => q.key !== key);
  autoInstalling.delete(key);
  kitItemDequeued(key);
}

export function clearQueue() {
  for (const q of store.queue) {
    autoInstalling.delete(q.key);
    kitItemDequeued(q.key);
  }
  store.queue = [];
}

/** Prefer showing a prerequisite the user can retry over one already in the queue. */
export const queueBlocker = (job: Queued) => job.prerequisites.find(key => !isPending(key)) ?? job.prerequisites[0];

export function retryPrerequisite(key: string) {
  const app = appByKey(key);
  if (app && !isPending(key)) install(app);
}

/** Pick the first runnable operation. A blocked branch keeps its place while
 *  independent work runs; a successful retry releases all its dependents. */
async function pump() {
  if (pumping) return;
  pumping = true;
  try {
    for (;;) {
      const index = store.queue.findIndex(job => !job.prerequisites.length || !appByKey(job.key));
      if (index === -1) break;
      const [next] = store.queue.splice(index, 1);
      activeRequest = next;
      const app = appByKey(next.key);
      const canRun = next.trigger === 'user' || verifiedKeys.has(next.key);
      if (app && canRun && (next.trigger !== 'autoInstall' || verifiedInstall(app))) {
        const ok = await runOperation(app, next.action, next.trigger, next.commit);
        if (ok && next.action !== 'uninstall') releaseDependents(store.queue, next.key);
        if (next.trigger === 'update' && !ok) declinedUpdates.add(next.key);
      } else {
        autoInstalling.delete(next.key);
        kitItemDequeued(next.key);
      }
      activeRequest = undefined;
      await recheckBlocked();
    }
  } finally {
    activeRequest = undefined;
    pumping = false;
  }
  if (!store.queue.length) maybeApplySelfUpdate();
}

// --- Kits

export const isInstalled = (key: string) => store.status[key] === 'installed' || store.status[key] === 'needsUpdate';
export const isPending = (key: string) => activeRequest?.key === key || store.queue.some((q) => q.key === key);
export const pendingUninstall = (key: string) => (activeRequest?.key === key && activeRequest.action === 'uninstall')
  || store.queue.some(q => q.key === key && q.action === 'uninstall');
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

const externalKeys = {
  nixos: ['app.status.nixos', 'app.declaredNixos'],
  'home-manager': ['app.status.homeManager', 'app.declaredHomeManager'],
  other: ['app.status.other', 'app.installedOutside'],
} as const;
/** The status of an app installed outside furnish, e.g. "Installed via Home Manager". */
export const externalStatus = (e: api.External) => t(externalKeys[e.by][0]);
/** Why furnish leaves an app installed outside it alone, and how to uninstall it. */
export const externalNote = (e: api.External) => t(externalKeys[e.by][1]);

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

/** A choice label in the user's language, or the literal option value. */
export function optionValueLabel(
  collection: string, option: api.Option, value: unknown, label = option.choices.find(c => c.value === value)?.label,
): string {
  if (value == null) return t('options.notSet');
  return choiceLabel(collection, option, value, label)
    ?? (value === true ? t('common.yes') : value === false ? t('common.no') : value === '' ? '""' : String(value));
}

/** A kit counts as installed when all its required items are. */
function kitInstalled(kit: KitInfo) {
  return kit.items.some((i) => isInstalled(i.key)) && kit.items.filter((i) => i.required).every((i) => isInstalled(i.key));
}

/** Installed apps and complete kits left behind when the selected apps are removed. */
export function removalImpact(keys: string[], removingKit?: string) {
  const removing = new Set(keys);
  return {
    apps: store.apps.filter(app => isInstalled(app.key) && !removing.has(app.key)
      && app.installedRequirements.some(key => removing.has(key))),
    kits: store.kits.filter(kit => kit.key !== removingKit && kitInstalled(kit)
      && kit.items.some(item => item.required && removing.has(item.key))),
  };
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

/** Attach the same queued/running operation to every kit that requested it. */
function trackKitItem(kit: KitActivity, key: string, scheduled: boolean) {
  let item = kit.items.find(i => i.key === key);
  if (!item) {
    const app = appByKey(key);
    kit.items.push({ key, name: app ? appName(app) : key, selected: false, state: 'queued', attempts: [] });
    item = kit.items[kit.items.length - 1];
  }
  if (!scheduled) {
    // Joining a blocked install can introduce a prerequisite which already failed.
    const previous = store.runs[key];
    item.state = previous?.result === 'failed' ? 'failed' : 'cancelled';
    return;
  }
  // Another selected app in this kit can reach the same prerequisite.
  if (item.ownsJob === undefined || !kitItemActive(item)) item.ownsJob = !isPending(key);
  const live = store.runs[key];
  if (activeRequest?.key === key && live) {
    if (live.result) {
      // The engine has finished but its post-operation status refresh is still pending.
      item.state = live.result;
      if (!item.attempts.some(attempt => attempt.summary.id === live.id)) {
        item.attempts.unshift(completedActivity(key, live, store.logs[key] ?? []));
      }
    } else {
      item.state = 'running';
      item.runId = live.id;
    }
  } else {
    item.state = 'queued';
  }
}

function kitsWaitingFor(key: string) {
  const dependents = store.queue.filter(job => job.prerequisites.includes(key));
  return store.recent.filter(isKit).filter(kit => kit.items.some(i =>
    kitItemActive(i) && dependents.some(job => job.key === i.key)) && kit.items.some(i => i.key === key));
}

/** A prerequisite retried from any entry point still belongs to its waiting kits. */
function kitItemRetried(key: string) {
  for (const kit of kitsWaitingFor(key)) trackKitItem(kit, key, true);
}

/** A live check can satisfy a prerequisite installed outside this request. */
function kitPrerequisiteAvailable(key: string) {
  const groups = kitsWaitingFor(key);
  for (const kit of groups) {
    const item = kit.items.find(i => i.key === key)!;
    item.state = 'done';
    kit.updatedAt = Date.now();
  }
  if (groups.length) saveActivity();
}

function kitItemDequeued(key: string) {
  for (const kit of store.recent.filter(isKit)) {
    const item = kit.items.find(i => i.key === key && i.state === 'queued');
    if (item) {
      item.state = 'cancelled';
      kit.updatedAt = Date.now();
    }
  }
  saveActivity();
}

function queueKitApp(kit: KitActivity, app: AppInfo) {
  const queued = install(app, defaultDependencies(app), 'user', (key, scheduled) => trackKitItem(kit, key, scheduled));
  if (!queued) {
    const item = kit.items.find(i => i.key === app.key);
    if (item) item.state = 'failed';
  }
  return queued;
}

/** One Activity result for the selected kit apps, including those already installed. */
export function installKit(kit: KitInfo, keys: string[]) {
  const conflict = kit.items.find(i => i.required && pendingUninstall(i.key));
  if (conflict) {
    const app = appByKey(conflict.key);
    toast('error', t('queue.uninstalling', { detail: app ? appName(app) : conflict.key }));
    return;
  }
  const selected = new Set(keys);
  const items: KitActivity['items'] = [];
  for (const { key } of kit.items) {
    if (!selected.delete(key) || pendingUninstall(key)) continue;
    const app = appByKey(key);
    items.push({ key, name: app ? appName(app) : key, selected: true,
      state: !app ? 'failed' : isInstalled(key) ? 'done' : 'queued', attempts: [] });
  }
  if (!items.length) return;
  store.recent.unshift({ id: crypto.randomUUID(), key: kit.key, name: kitName(kit), updatedAt: Date.now(), items });
  const group = store.recent[0] as KitActivity;
  for (const item of group.items.filter(i => i.selected && i.state === 'queued')) {
    const app = appByKey(item.key);
    if (app) queueKitApp(group, app);
  }
  saveActivity();
}

/** The queue is session-only. Reattach a saved dependent to its prerequisite without
 *  restarting other interrupted or cancelled jobs. Respect today's catalog and reuse
 *  any install that is already underway. */
function restoreKitDependent(kit: KitActivity, item: KitActivity['items'][number]) {
  const app = appByKey(item.key);
  if (!app || store.status[item.key] === 'unavailable') return;
  if (!isPending(item.key) && verifiedKeys.has(item.key) && isInstalled(item.key)) {
    item.state = 'done';
    return;
  }
  const pending = activeRequest ? [activeRequest, ...store.queue] : store.queue;
  const selected = defaultDependencies(app);
  for (const key of item.waitingFor ?? []) {
    if (!selected.includes(key) && app.dependencies.some(dep => dep.key === key)) selected.push(key);
  }
  const plan = planInstall(app, selected, store.apps, pending, key => isInstalled(key) || store.status[key] === 'unavailable');
  if (plan.error) {
    item.state = 'failed';
    toast('error', t(`queue.${plan.error.kind}`, { detail: plan.error.detail }));
    return;
  }
  for (const key of plan.members) {
    if (key === item.key || pending.some(job => job.key === key)) trackKitItem(kit, key, true);
    else if (!kit.items.some(i => i.key === key)) {
      const dependency = appByKey(key);
      kit.items.push({ key, name: dependency ? appName(dependency) : key, selected: false, state: 'interrupted', attempts: [] });
    }
  }
  // Only this dependent is being resumed. Its other outstanding edges still wait
  // for their prerequisites to be verified or explicitly retried.
  const job = plan.installs.find(j => j.key === item.key);
  if (job) store.queue.push({ ...job, trigger: 'user', action: 'install' });
  pump();
}

export function retryKitItem(kit: KitActivity, key: string) {
  const item = kit.items.find(i => i.key === key);
  const app = appByKey(key);
  if (!app || !item || !['cancelled', 'failed', 'interrupted'].includes(item.state) || isPending(key)
    || store.status[key] === 'unavailable') return;
  kit.updatedAt = Date.now();
  const interrupted = interruptedDependents(kit, key);
  if (queueKitApp(kit, app)) {
    for (const dependent of interrupted) restoreKitDependent(kit, dependent);
  }
  saveActivity();
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
