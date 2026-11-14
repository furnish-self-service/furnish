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
  /** Keys of apps installed first, e.g. a package manager it needs. */
  requires: string[];
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
  items: { key: string; required: boolean; selected: boolean }[];
}

export interface CollectionInfo {
  name: string;
  url: string;
  revision: string;
  apps: number;
}

export const listApps = () => invoke<AppInfo[]>('list_apps');
export const listKits = () => invoke<KitInfo[]>('list_kits');
/** Status; for 'blocked', `reason` says what is missing, e.g. "Homebrew (brew)". */
export const appStatus = (key: string) => invoke<{ status: Status; reason: string | null }>('app_status', { key });
export const install = (key: string) => invoke<void>('install', { key });
export const uninstall = (key: string) => invoke<void>('uninstall', { key });
export const listCollections = () => invoke<CollectionInfo[]>('list_collections');
export const addCollection = (url: string, name: string | null) =>
  invoke<CollectionInfo>('add_collection', { url, name });
export const updateCollection = (name: string) =>
  invoke<{ old: string; new: string }>('update_collection', { name });
export const removeCollection = (name: string) => invoke<void>('remove_collection', { name });

export interface Settings {
  /** Pull all collections when the app starts. */
  updateOnLaunch: boolean;
  /** Pull all collections this often while the app runs; 0 turns it off. */
  updateIntervalMinutes: number;
}
export const getSettings = () => invoke<Settings>('get_settings');
export const saveSettings = (settings: Settings) => invoke<void>('save_settings', { settings });
