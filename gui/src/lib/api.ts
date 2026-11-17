// Typed wrappers around the Rust commands in src-tauri/src/main.rs.
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

export type Status = 'installed' | 'notInstalled' | 'needsUpdate' | 'unavailable' | 'blocked';

/** A resource that applies on this computer with the current options. */
export interface Resource {
  name: string;
}

export interface AppInfo {
  key: string;
  id: string;
  collection: string;
  /** The name to show (`metadata.furnish.name`); null: the id. See `appName` for translations. */
  name: string | null;
  description: string;
  categories: string[];
  icon: string | null;
  /** Shown instead of `icon` in dark mode (`metadata.furnish.iconDark`). */
  iconDark: string | null;
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
  /** The same as notes to translate (furnish's configuration resources): `plan.note.<code>`. */
  notes?: { code: string; params: Record<string, unknown> }[];
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
  name: string | null;
  description: string;
  categories: string[];
  icon: string | null;
  /** Shown instead of `icon` in dark mode (`iconDark`). */
  iconDark: string | null;
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
  /** Data URL of the avatar for dark mode (`iconDark`). */
  iconDark: string | null;
  /** History kept, downloads and falling back, and which of them the organization sets. */
  settings: { settings: CollectionSettings; locked: (keyof CollectionSettings)[] };
  /** The language of the collection's own texts. */
  language: string;
  /** Its translations (`locales/<lang>.yaml`) by language tag. */
  locales: Record<string, import('./content.ts').LocaleFile>;
  /** The server had a newer commit than the one checked out, when furnish last looked. */
  updateAvailable: boolean;
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
/** `external`: installed outside furnish (NixOS, Home Manager, another tool); furnish leaves it alone. */
export const appStatus = (key: string) =>
  invoke<{ status: Status; reason: string | null; removable: boolean; external: External | null }>('app_status', { key });
/**
 * A package installed outside furnish: by the NixOS or Home Manager configuration (`package` is its
 * nixpkgs attribute), or with another tool (`other`, e.g. npm; `path` is where its program is).
 */
export type External = { by: 'nixos' | 'home-manager' | 'other'; package: string; configuration: string | null; path: string | null };
/** How furnish found a NixOS or Home Manager configuration, or the flake attribute. */
export type NixosSource = 'setting' | 'module' | 'policy' | 'etc-nixos' | 'nh' | 'nix-path' | 'hostname' | 'env' | 'usual-place';
export type NixosProblem = 'notFound' | 'noNixosConfigurations' | 'noAttribute';
export type NixosFound = {
  path: string;
  kind: 'flake' | 'channels';
  source: NixosSource;
  via: string | null;
  /** 'differs': not the commit the running system was built from (still used). */
  revision: 'unknown' | 'matches' | 'differs';
  /** For a path someone set: what the check objects to. */
  problem: NixosProblem | null;
};
/** A value of `osSpecific.nixos` set by the user or the organization. */
export type NixosLeaf = { value: string; setBy: 'user' | 'organization'; locked: boolean } | null;
/** How this NixOS computer is set up; null on other systems. */
export type NixosSetup = {
  configuration: NixosFound | null;
  flakeAttribute: { value: string; source: NixosSource };
  homeManagerConfiguration: NixosFound | null;
  homeManager: boolean;
  /** Folders with a flake.nix, only when nothing was found. */
  suggestions: { path: string; problem: NixosProblem | null }[];
  /** A configuration path set that has no configuration, and why. */
  rejected: [string, NixosProblem] | null;
  chosen: { configuration: NixosLeaf; flakeAttribute: NixosLeaf; homeManagerConfiguration: NixosLeaf };
  /** Set up with furnish's NixOS module: its `services.furnish.nix` values. */
  module: { nix: { configuration: string | null; flakeAttribute: string | null; allowedOptions: string[]; rebuild: string | null } } | null;
  /** Its flake.nix points furnish/apps at furnish's local flake: furnish can install apps into it. */
  appsInput: boolean;
};
export const nixosSetup = () => invoke<NixosSetup | null>('nixos_setup');
/** Set `osSpecific.nixos.<name>`; null returns to finding it automatically. */
export const setNixosSetting = (name: 'configuration' | 'flakeAttribute' | 'homeManagerConfiguration', value: string | null) =>
  invoke<void>('set_setting', { key: `osSpecific.nixos.${name}`, value });
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

/** How furnish was installed: furnish installs its own updates only for `msi` and `app`. */
export type InstallMethod = 'msi' | 'app' | 'homebrew' | 'deb' | 'rpm' | 'nix' | 'other';
/** furnish's own updates (src-tauri/src/selfupdate.rs). */
export interface SelfUpdateStatus {
  method: InstallMethod;
  /** The setting as it applies here: `auto` only where furnish installs its updates. */
  mode: 'auto' | 'notify' | 'off';
  locked: boolean;
  current: string;
  /** A newer version the last check found. */
  available: string | null;
  notes: string | null;
  /** `ready`: downloaded and checked, installs and restarts when nothing is installing. */
  state: 'idle' | 'checking' | 'downloading' | 'ready';
  error: string | null;
}
export const selfUpdateStatus = () => invoke<SelfUpdateStatus>('self_update_status');
/** Ask for a newer version; `download`: fetch it now even if only notifying (Update now). */
export const checkSelfUpdate = (download = false) => invoke<SelfUpdateStatus>('check_self_update', { download });
/** Install the downloaded version and restart, unless something installs (or, with
 *  `onlyHidden`, the window is open). Resolves to whether it did. */
export const applySelfUpdate = (onlyHidden: boolean) => invoke<boolean>('apply_self_update', { onlyHidden });
export const onSelfUpdate = (handler: (s: SelfUpdateStatus) => void) =>
  listen<SelfUpdateStatus>('furnish://self-update', (e) => handler(e.payload));
/** Restart to update, chosen in the tray menu. */
export const onRestartToUpdate = (handler: () => void) => listen('furnish://restart-to-update', () => handler());
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
/** Ask each collection's server for its newest commit, without applying it (see updateAvailable). */
export const checkCollectionServers = () => invoke<void>('check_collection_servers');
/** The folder furnish keeps its collections, settings and logs in. */
export const dataDir = () => invoke<string>('data_dir');
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
  /** Collection texts not translated into the app's language show in this one (a language tag). */
  fallbackLanguage: string;
  /** New versions of furnish: install them where furnish can, only say so, or don't check. */
  selfUpdate: 'auto' | 'notify' | 'off';
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
  invoke<{ settings: Settings; locked: string[]; usersCanAddCollections: boolean; warnings: import('./i18n.ts').SettingsWarning[] }>(
    'get_settings',
  );
export const setSetting = (key: keyof Settings, value: unknown) => invoke<void>('set_setting', { key, value });

/** The tray menu's texts; `status`: updates waiting or what furnish is doing, empty for nothing. */
export interface TrayTexts {
  open: string;
  check: string;
  settings: string;
  quit: string;
  /** Shown while a new version of furnish waits to be installed. */
  restart: string;
  status: string;
  tooltip: string;
}
export const setTray = (texts: TrayTexts) => invoke<void>('set_tray', { texts });
/** Check for updates, chosen in the tray menu. */
export const onCheckUpdates = (handler: () => void) => listen('furnish://check-updates', () => handler());
/** Settings…, chosen in the tray menu (the window is shown already). */
export const onOpenSettings = (handler: () => void) => listen('furnish://open-settings', () => handler());
/** The window was closed, and furnish keeps running in the tray. */
export const onToTray = (handler: () => void) => listen('furnish://to-tray', () => handler());
/** The window was opened again (tray, starting furnish again). */
export const onShown = (handler: () => void) => listen('furnish://shown', () => handler());
/** Clone the admin's preset collections; returns problems (e.g. offline). */
export const ensurePresets = () => invoke<string[]>('ensure_presets');
/** Fetch the organizations' descriptions again and add the collections new in those of addresses
 *  the user added. Returns problems (descriptions that fail the checks keep the old ones) and the
 *  added collections. */
export const refreshEndpoints = () => invoke<{ problems: string[]; added: string[] }>('refresh_endpoints');
