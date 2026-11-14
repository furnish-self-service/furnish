// Typed wrappers around the Rust commands in src-tauri/src/main.rs.
import { invoke } from '@tauri-apps/api/core';

export type Status = 'installed' | 'notInstalled' | 'needsUpdate' | 'unavailable';

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
}

export interface CollectionInfo {
  name: string;
  url: string;
  revision: string;
  apps: number;
}

export const listApps = () => invoke<AppInfo[]>('list_apps');
export const appStatus = (key: string) => invoke<Status>('app_status', { key });
export const install = (key: string) => invoke<void>('install', { key });
export const uninstall = (key: string) => invoke<void>('uninstall', { key });
export const listCollections = () => invoke<CollectionInfo[]>('list_collections');
export const addCollection = (url: string, name: string | null) =>
  invoke<CollectionInfo>('add_collection', { url, name });
export const updateCollection = (name: string) =>
  invoke<{ old: string; new: string }>('update_collection', { name });
export const removeCollection = (name: string) => invoke<void>('remove_collection', { name });
