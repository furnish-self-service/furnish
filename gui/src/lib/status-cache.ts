// Last successful GUI checks. Each app is written separately so closing during a scan
// keeps completed checks without rewriting the whole catalog for every result.
import type { AppCheck, External, Required } from './api';
import type { Entry } from './required';

const object = (v: unknown): v is Record<string, unknown> => v !== null && typeof v === 'object';
const nullableString = (v: unknown) => v === null || typeof v === 'string';
const oneOf = (v: unknown, choices: string[]) => typeof v === 'string' && choices.includes(v);

function external(v: unknown): v is External {
  return object(v) && oneOf(v.by, ['nixos', 'home-manager', 'other'])
    && typeof v.package === 'string' && nullableString(v.configuration) && nullableString(v.path);
}

function need(v: unknown): v is Entry {
  if (!object(v)) return false;
  if (v.kind === 'regex') return typeof v.pattern === 'string';
  return v.kind === 'range' && Array.isArray(v.all) && v.all.every(c =>
    object(c) && oneOf(c.op, ['>=', '>', '<=', '<', '=', '!=']) && typeof c.version === 'string');
}

function required(v: unknown): v is Required {
  return object(v) && typeof v.resource === 'string' && nullableString(v.installed)
    && nullableString(v.offered) && nullableString(v.target) && typeof v.via === 'string' && typeof v.summary === 'string'
    && typeof v.acceptable === 'boolean' && typeof v.installer === 'boolean' && typeof v.unknown === 'boolean'
    && Array.isArray(v.needs) && v.needs.every(need);
}

function valid(v: unknown): v is AppCheck {
  return object(v) && oneOf(v.status, ['installed', 'notInstalled', 'needsUpdate', 'unavailable', 'blocked'])
    && nullableString(v.reason) && typeof v.removable === 'boolean'
    && (v.external === null || external(v.external)) && (v.required === null || required(v.required));
}

interface SavedCheck {
  check: AppCheck;
  /** Older cache records predate this field. */
  checkedAt: number | null;
}

/** A changed or unknown revision can restore Installed membership, but its plan details are stale. */
export function restoreCheck(entry: unknown, revision: string | undefined): { installed: boolean; saved?: SavedCheck } | undefined {
  if (!object(entry) || !valid(entry.check)) return;
  const installed = entry.check.status === 'installed' || entry.check.status === 'needsUpdate';
  const time = entry.checkedAt;
  const checkedAt = typeof time === 'number' && Number.isSafeInteger(time) && time > 0 && time <= 8.64e15 ? time : null;
  const saved = revision && revision !== 'unknown' && revision === entry.revision ? { check: entry.check, checkedAt } : undefined;
  return { installed, saved };
}

export class StatusCache {
  private prefix: string;

  constructor(dataDir: string) {
    this.prefix = 'furnish.appStatus.v1:' + JSON.stringify(dataDir) + ':';
  }

  entry(key: string): unknown {
    try {
      const raw = localStorage.getItem(this.prefix + key);
      if (raw !== null) return JSON.parse(raw);
    } catch { /* Unavailable storage or an incomplete entry: use the live check. */ }
  }

  write(key: string, revision: string, check: AppCheck, checkedAt: number) {
    try {
      localStorage.setItem(this.prefix + key, JSON.stringify({ revision, check, checkedAt }));
    } catch { /* Checking works even when the webview cannot save data. */ }
  }

  prune(appKeys: Set<string>) {
    try {
      for (let i = localStorage.length - 1; i >= 0; i--) {
        const name = localStorage.key(i);
        if (name?.startsWith(this.prefix) && !appKeys.has(name.slice(this.prefix.length))) localStorage.removeItem(name);
      }
    } catch { /* Best effort, as for reads and writes. */ }
  }
}
