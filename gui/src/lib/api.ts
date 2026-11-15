// Typed wrappers around the Rust commands in src-tauri/src/main.rs.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type Status = 'installed' | 'notInstalled' | 'needsUpdate' | 'unavailable' | 'blocked';

export interface Resource {
  name: string;
  type: string;
  condition: string | null;
}

export interface AppInfo {
  key: string;
  id: string;
  collection: string;
  description: string;
  categories: string[];
  icon: string | null;
  resources: Resource[];
  /** The app's DSC configuration document, as written in the collection. */
  source: string;
  /** Apps installed first: required ones always, optional ones if the user keeps them selected. */
  dependencies: Item[];
  /** Options the app declares, with their current values. */
  options: Option[];
  /** Installed with other option values than the current ones: a reinstall applies them. */
  optionsDiffer: boolean;
  /** Recommended for this user (the collection decides, e.g. by role). */
  recommended: boolean;
  /** Names of required collection options nobody answered yet: installing waits for them. */
  unanswered: string[];
  /** Held at this earlier version because the collection's current one failed. */
  held: Version | null;
}

/** A version of an app: a commit of its collection, and the day of it. */
export interface Version {
  commit: string;
  date: string;
}

/** Who started an operation, for reports: the user (default), automatic updates, or the organization's first-start installs. */
export type Trigger = 'user' | 'update' | 'autoInstall';
/** The organization's furnish addresses that take reports. */
export const reportDestinations = () => invoke<string[]>('report_destinations');
/** The installed apps for the daily inventory report (ignored unless due). */
export const reportInventory = (apps: [string, string][]) => invoke<void>('report_inventory', { apps });

/** An install failed and furnish installed an earlier version instead (fallback "automatic"). */
export interface FellBack {
  date: string;
  error: string;
}

/** An option (DSC parameter) of an app or collection and where its value comes from. */
export interface Option {
  name: string;
  type: 'bool' | 'string' | 'int';
  default?: unknown;
  label?: string;
  description?: string;
  /** The allowed values with what to show; empty: any value of the type. */
  choices: { value: unknown; label: string | null }[];
  /** Users must choose a value. */
  required?: boolean;
  value: unknown;
  source: 'policy' | 'app' | 'collection' | 'default';
  /** The admin policy fixes the value. */
  locked: boolean;
  /** The user's own value at this level, if any. */
  ownValue: unknown;
  /** Required, but nobody chose a value yet. */
  missing: boolean;
}

/** An app a kit contains or an app depends on. */
export interface Item {
  key: string;
  required: boolean;
  /** Preselected when installing (required items always are). */
  selected: boolean;
}

export interface LogLine {
  level: 'error' | 'warn' | 'info' | 'output';
  message: string;
}

export type Action = 'install' | 'remove' | 'change' | 'nothing' | 'skipped';

export interface Step {
  name: string;
  type: string;
  action: Action;
  /** The resource's own description of what it would do, e.g. the exact command. */
  details: string[];
  changes: { property: string; before: unknown; after: unknown }[];
}

export interface Outcome {
  action: Action;
  changes: Step['changes'];
  /** Set if the step failed; the run stops there. */
  error: string | null;
  /** The user cancelled this step (refused the password dialog or force-stopped it). */
  cancelled: boolean;
}

/** Error message of an install/update/uninstall the user cancelled. */
export const CANCELLED = 'cancelled';
/** Stop the running operation after its current step. */
export const abort = () => invoke<void>('abort');
/** Stop the running operation now, interrupting its current step. */
export const forceStop = () => invoke<void>('force_stop');

export type Progress = { key: string } & (
  | { event: 'planned'; steps: Step[] }
  | { event: 'started'; index: number; atMs: number }
  | { event: 'finished'; index: number; atMs: number; durationMs: number; outcome: Outcome }
);

export const onProgress = (handler: (p: Progress) => void) =>
  listen<Progress>('furnish://progress', (event) => handler(event.payload));

export const plan = (key: string, uninstall: boolean) => invoke<Step[]>('plan', { key, uninstall });

export const onLog = (handler: (line: LogLine) => void) =>
  listen<LogLine>('furnish://log', (event) => handler(event.payload));

export interface KitInfo {
  /** `collection/id`; kits and apps share names. */
  key: string;
  id: string;
  collection: string;
  description: string;
  categories: string[];
  icon: string | null;
  items: Item[];
}

export interface CollectionInfo {
  name: string;
  /** Provided by the admin policy: can't be removed. */
  managed: boolean;
  url: string;
  revision: string;
  apps: number;
  /** Options shared by the collection's apps. */
  options: Option[];
  /** Data URL of the collection's avatar. */
  icon: string | null;
  /** History kept, downloads and falling back, and which of them the organization sets. */
  settings: { settings: CollectionSettings; locked: (keyof CollectionSettings)[] };
}

export interface CollectionSettings {
  /** Commits of history to keep; 0: all. */
  checkoutDepth: number;
  /** Download file contents when needed: installer files when their app is installed. */
  blobsOnDemand: boolean;
  /** When an install fails: offer the previous version, install it, or neither. */
  fallback: 'ask' | 'automatic' | 'off';
  /** Only versions at most this many days older than the failing one; 0: any age. */
  fallbackMaxAgeDays: number;
}

/** The organization's name, logo and colors (admin policy `branding`). */
export const getBranding = () => invoke<import('./brand').Branding>('get_branding');
export const listApps = () => invoke<AppInfo[]>('list_apps');
export const listKits = () => invoke<KitInfo[]>('list_kits');
/** Status; for 'blocked', `reason` says what is missing, e.g. "Homebrew (brew)". */
/** `removable`: furnish can uninstall it here (no resource that can't remove, or removal steps). */
export const appStatus = (key: string) => invoke<{ status: Status; reason: string | null; removable: boolean }>('app_status', { key });
export const install = (key: string, trigger?: Trigger) => invoke<FellBack | null>('install', { key, trigger: trigger ?? null });
export const uninstall = (key: string, trigger?: Trigger) => invoke<FellBack | null>('uninstall', { key, trigger: trigger ?? null });
export const reinstall = (key: string, trigger?: Trigger) => invoke<FellBack | null>('reinstall', { key, trigger: trigger ?? null });
/** Install the previous version: the one that last installed here, else the one before the latest change. */
export const installPrevious = (key: string, trigger?: Trigger) => invoke<FellBack | null>('install_previous', { key, trigger: trigger ?? null });
/** For an app held at an earlier version: install the collection's current one. */
export const installCurrent = (key: string, trigger?: Trigger) => invoke<FellBack | null>('install_current', { key, trigger: trigger ?? null });
/** The version furnish would fall back to; rejects with the reason if there is none. */
export const previousVersion = (key: string) => invoke<Version>('previous_version', { key });
export const version = () => invoke<string>('version');
/** Open the list of open-source components and their licenses in the browser. */
export const openLicenses = () => invoke<void>('open_licenses');
/** Change a setting for one collection; null returns it to the general setting. */
export const setCollectionSetting = (name: string, key: keyof CollectionSettings, value: unknown) =>
  invoke<void>('set_collection_setting', { name, key, value });
/** Set an option of a collection or an app (`collection/app`); null resets it. */
export const setOption = (target: string, name: string, value: unknown) =>
  invoke<void>('set_option', { target, name, value });
export const listCollections = () => invoke<CollectionInfo[]>('list_collections');
/** A git collection, or an organization's furnish address (furnish.myorg.tld): all its collections. */
export const addCollection = (url: string, name: string | null, username?: string, password?: string) =>
  invoke<CollectionInfo[]>('add_collection', { url, name, username: username || null, password: password || null });
/** The organization's furnish addresses this computer knows, and whether the user is signed in. */
export interface Endpoint {
  origin: string;
  name: string;
  signedIn: boolean;
}
export const endpoints = () => invoke<Endpoint[]>('endpoints');
/** Sign in again in the browser; returns problems setting up its collections. */
export const signIn = (origin: string) => invoke<string[]>('sign_in', { origin });
/** The sign-in page's URL, in case the browser doesn't open. */
export interface SignInStarted {
  url: string;
  /** The furnish address the sign-in is for. */
  origin: string;
}
export const onSignIn = (handler: (started: SignInStarted) => void) =>
  listen<SignInStarted>('furnish://sign-in', (e) => handler(e.payload));
/** What adding a furnish address from a link would do (the link must name a furnish address). */
export interface LinkPreview {
  /** The address, normalized: https://furnish.myorg.tld */
  origin: string;
  name: string;
  /** Where the user enters their password. */
  signInHost: string;
  collections: string[];
  /** Apps it installs right away. */
  autoInstall: string[];
  /** Reports it turns on: errors or all; null: none. */
  reporting: 'errors' | 'all' | null;
  /** Whether it gives furnish its name and colors. */
  branding: boolean;
}
export const linkPreview = (link: string) => invoke<LinkPreview>('link_preview', { link });
/** Add a furnish address the user confirmed: sign in, then add its collections. */
export const addEndpoint = (origin: string) => invoke<CollectionInfo[]>('add_endpoint', { origin });
/** furnish:// links (e.g. furnish://add?endpoint=furnish.myorg.tld): the one furnish started with, then events. */
export const pendingLink = () => invoke<string | null>('pending_link');
export const onLink = (handler: (url: string) => void) => listen<string>('furnish://link', (e) => handler(e.payload));
/** Apps to install on first launch (organization's choice), not installed that way yet. */
export const autoInstallPending = () => invoke<string[]>('auto_install_pending');
export const autoInstallDone = (app: string) => invoke<void>('auto_install_done', { app });
export const updateCollection = (name: string) =>
  invoke<{ old: string; new: string }>('update_collection', { name });
export const removeCollection = (name: string) => invoke<void>('remove_collection', { name });

export interface Settings {
  /** Start at sign-in, hidden in the tray. */
  startAtSignIn: boolean;
  /** Pull all collections when the app starts. */
  updateOnLaunch: boolean;
  /** Pull all collections this often while the app runs; 0 turns it off. */
  updateIntervalMinutes: number;
  /** Apps (`collection/app`) whose updates are not installed automatically. */
  noAutoUpdate: string[];
  /** Postpone automatic updates while the computer is busy. */
  loadAwareUpdates: boolean;
  loadLimits: { cpuPercent: number; memoryPercent: number; networkKbps: number };
  /** Reports to the organization: off, errors (failures and fallbacks), all. */
  reporting: 'off' | 'errors' | 'all';
  /** Include this computer's name in reports. */
  reportHostname: boolean;
  /** The app's language; `system` follows the system language. */
  language: 'system' | import('./i18n.ts').Lang;
}

export interface Load {
  average: { cpuPercent: number; memoryPercent: number; networkKbps: number };
  samples: number;
  /** Why automatic updates should wait, e.g. "CPU 84%"; null: go ahead. */
  busy: string | null;
  retryAfterSeconds: number;
}
export const systemLoad = () => invoke<Load>('system_load');
/** Settings in effect plus what the admin policy restricts. */
export const getSettings = () =>
  invoke<{ settings: Settings; locked: string[]; usersCanAddCollections: boolean }>('get_settings');
export const setSetting = (key: keyof Settings, value: unknown) => invoke<void>('set_setting', { key, value });

/** The tray menu's texts; `status`: updates waiting or what furnish is doing, empty for nothing. */
export interface TrayTexts {
  open: string;
  check: string;
  quit: string;
  status: string;
  tooltip: string;
}
export const setTray = (texts: TrayTexts) => invoke<void>('set_tray', { texts });
/** Check for updates, chosen in the tray menu. */
export const onCheckUpdates = (handler: () => void) => listen('furnish://check-updates', () => handler());
/** The window was closed, and furnish keeps running in the tray. */
export const onToTray = (handler: () => void) => listen('furnish://to-tray', () => handler());
/** The window was opened again (tray, starting furnish again). */
export const onShown = (handler: () => void) => listen('furnish://shown', () => handler());
/** Clone the admin's preset collections; returns problems (e.g. offline). */
export const ensurePresets = () => invoke<string[]>('ensure_presets');
