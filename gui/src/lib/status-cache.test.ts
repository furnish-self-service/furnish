import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { AppCheck } from './api';
import { StatusCache } from './status-cache.ts';

const checkedAt = 1_800_000_000_000;
const installed: AppCheck = {
  status: 'installed', removable: false, reason: null, required: null,
  external: { by: 'home-manager', package: 'jq', configuration: '/etc/home', path: null },
};

function storage() {
  const entries = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    get length() { return entries.size; },
    key: (i: number) => [...entries.keys()][i] ?? null,
    getItem: (key: string) => entries.get(key) ?? null,
    setItem: (key: string, value: string) => entries.set(key, value),
    removeItem: (key: string) => entries.delete(key),
  } });
  return entries;
}

test('a fresh launch restores completed checks and picks up later changes per app', () => {
  storage();
  const cache = new StatusCache('/profile');
  cache.write('team/jq', 'a25d340', installed, checkedAt);
  const launched = new StatusCache('/profile');
  assert.deepEqual(launched.read('team/jq', 'a25d340'), { check: installed, checkedAt });
  assert.equal(launched.read('team/new', 'a25d340'), undefined);
  assert.equal(launched.read('team/jq', '123beef'), undefined);
  assert.equal(new StatusCache('/other').read('team/jq', 'a25d340'), undefined);
  const removed: AppCheck = { ...installed, status: 'notInstalled', external: null, removable: true };
  launched.write('team/jq', 'a25d340', removed, checkedAt);
  assert.deepEqual(new StatusCache('/profile').read('team/jq', 'a25d340'), { check: removed, checkedAt });
});

test('restores the status from an older entry without inventing a verification time', () => {
  const entries = storage();
  const cache = new StatusCache('/profile');
  cache.write('team/jq', 'rev', installed, checkedAt);
  const [key] = entries.keys();
  entries.set(key, JSON.stringify({ revision: 'rev', check: installed }));
  assert.deepEqual(cache.read('team/jq', 'rev'), { check: installed, checkedAt: null });
});

test('pruning removed apps preserves active apps and other profiles', () => {
  storage();
  const cache = new StatusCache('/profile');
  const other = new StatusCache('/other');
  cache.write('team/jq', 'rev', installed, checkedAt);
  cache.write('other/jq', 'rev', installed, checkedAt);
  other.write('team/jq', 'rev', installed, checkedAt);
  cache.prune(new Set(['other/jq']));
  assert.equal(cache.read('team/jq', 'rev'), undefined);
  assert.deepEqual(cache.read('other/jq', 'rev'), { check: installed, checkedAt });
  assert.deepEqual(other.read('team/jq', 'rev'), { check: installed, checkedAt });
});

test('unreadable or incompatible cached checks leave the app ready for a live check', () => {
  const entries = storage();
  const cache = new StatusCache('/profile');
  cache.write('team/jq', 'rev', installed, checkedAt);
  const [key] = entries.keys();
  for (const contents of [
    'truncated',
    JSON.stringify({ revision: 'rev', check: { ...installed, status: ['installed'] } }),
    JSON.stringify({ revision: 'rev', check: { ...installed, required: { needs: null } } }),
  ]) {
    entries.set(key, contents);
    assert.equal(cache.read('team/jq', 'rev'), undefined);
  }
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('unavailable'); } });
  cache.write('team/jq', 'rev', installed, checkedAt);
  assert.equal(cache.read('team/jq', 'rev'), undefined);
  cache.prune(new Set());
});
