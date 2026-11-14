// App-wide state and the actions that change it. Components read `store` and call these.
import * as api from './api';
import type { AppInfo, CollectionInfo, LogLine, Status } from './api';

export type View =
  | { page: 'discover' }
  | { page: 'installed' }
  | { page: 'updates' }
  | { page: 'category'; name: string }
  | { page: 'collections' }
  | { page: 'app'; key: string; back: View };

export type Busy = 'install' | 'uninstall';

export interface Toast {
  id: number;
  kind: 'success' | 'error';
  title: string;
  detail?: string;
}

export const store = $state({
  apps: [] as AppInfo[],
  /** Missing key: status not known yet. 'error': the check failed. */
  status: {} as Record<string, Status | 'error'>,
  busy: {} as Record<string, Busy>,
  collections: [] as CollectionInfo[],
  loading: true,
  view: { page: 'discover' } as View,
  query: '',
  toasts: [] as Toast[],
  /** Activity log of the last install/update/uninstall per app (this session only). */
  logs: {} as Record<string, LogLine[]>,
});

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
  if (kind === 'success') setTimeout(() => dismiss(id), 4000);
}
export function dismiss(id: number) {
  store.toasts = store.toasts.filter((t) => t.id !== id);
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

export async function loadAll() {
  store.loading = true;
  // Loaded independently: a problem with one must not hide the other.
  const [apps, collections] = await Promise.allSettled([api.listApps(), api.listCollections()]);
  if (apps.status === 'fulfilled') store.apps = apps.value;
  else toast('error', "Couldn't load the apps", message(apps.reason));
  if (collections.status === 'fulfilled') store.collections = collections.value;
  else toast('error', "Couldn't load the collections", message(collections.reason));
  store.loading = false;
  store.status = {};
  // The engine runs one check at a time anyway; sequential keeps the UI updating card by card.
  for (const app of store.apps) await checkStatus(app.key);
}

export async function checkStatus(key: string) {
  try {
    store.status[key] = await api.appStatus(key);
  } catch {
    store.status[key] = 'error';
  }
}

export async function install(app: AppInfo) {
  const updating = store.status[app.key] === 'needsUpdate';
  store.logs[app.key] = [];
  store.busy[app.key] = 'install';
  try {
    await api.install(app.key);
    toast('success', `${updating ? 'Updated' : 'Installed'} ${app.id}`);
  } catch (e) {
    toast('error', `Couldn't ${updating ? 'update' : 'install'} ${app.id}`, message(e));
  } finally {
    delete store.busy[app.key];
    await checkStatus(app.key);
  }
}

export async function uninstall(app: AppInfo) {
  store.logs[app.key] = [];
  store.busy[app.key] = 'uninstall';
  try {
    await api.uninstall(app.key);
    toast('success', `Uninstalled ${app.id}`);
  } catch (e) {
    toast('error', `Couldn't uninstall ${app.id}`, message(e));
  } finally {
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
