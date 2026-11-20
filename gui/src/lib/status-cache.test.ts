import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { AppCheck } from './api';
import { restoreCheck, StatusCache } from './status-cache.ts';

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
  assert.deepEqual(restoreCheck(launched.entry('team/jq'), 'a25d340'), { ...installed, checkedAt, source: 'cached' });
  assert.equal(restoreCheck(launched.entry('team/new'), 'a25d340'), undefined);
  assert.equal(restoreCheck(launched.entry('team/jq'), '123beef')?.source, 'previousRevision');
  assert.equal(restoreCheck(new StatusCache('/other').entry('team/jq'), 'a25d340'), undefined);
  const removed: AppCheck = { ...installed, status: 'notInstalled', external: null, removable: true };
  launched.write('team/jq', 'a25d340', removed, checkedAt);
  assert.deepEqual(restoreCheck(new StatusCache('/profile').entry('team/jq'), 'a25d340'), { ...removed, checkedAt, source: 'cached' });
});

test('restores the status from an older entry without inventing a verification time', () => {
  const entries = storage();
  const cache = new StatusCache('/profile');
  cache.write('team/jq', 'rev', installed, checkedAt);
  const [key] = entries.keys();
  entries.set(key, JSON.stringify({ revision: 'rev', check: installed }));
  assert.deepEqual(restoreCheck(cache.entry('team/jq'), 'rev'), { ...installed, checkedAt: null, source: 'cached' });
});

test('pruning removed apps preserves active apps and other profiles', () => {
  storage();
  const cache = new StatusCache('/profile');
  const other = new StatusCache('/other');
  cache.write('team/jq', 'rev', installed, checkedAt);
  cache.write('other/jq', 'rev', installed, checkedAt);
  other.write('team/jq', 'rev', installed, checkedAt);
  cache.prune(new Set(['other/jq']));
  assert.equal(restoreCheck(cache.entry('team/jq'), 'rev'), undefined);
  assert.deepEqual(restoreCheck(cache.entry('other/jq'), 'rev'), { ...installed, checkedAt, source: 'cached' });
  assert.deepEqual(restoreCheck(other.entry('team/jq'), 'rev'), { ...installed, checkedAt, source: 'cached' });
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
    assert.equal(restoreCheck(cache.entry('team/jq'), 'rev'), undefined);
  }
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, get() { throw new Error('unavailable'); } });
  cache.write('team/jq', 'rev', installed, checkedAt);
  assert.equal(restoreCheck(cache.entry('team/jq'), 'rev'), undefined);
  cache.prune(new Set());
});

test('changed and unknown revisions restore each last observed status for display', () => {
  for (const status of ['installed', 'needsUpdate', 'notInstalled', 'blocked', 'unavailable']) {
    const check = { ...installed, status };
    const entry = { revision: 'previous', checkedAt, check: { ...check, runningApps: ['closed since check'] } };
    for (const revision of ['next', 'unknown', undefined]) {
      assert.deepEqual(restoreCheck(entry, revision), { ...check, checkedAt, source: 'previousRevision' });
    }
  }
});

test('the newest readable observation wins across native and webview storage', () => {
  const disk = { revision: 'rev', checkedAt, check: installed };
  const web = { ...disk, checkedAt: checkedAt + 1, check: { ...installed, status: 'needsUpdate' } };
  assert.equal(restoreCheck(disk, 'rev', web)?.status, 'needsUpdate');
  assert.equal(restoreCheck(web, 'rev', disk)?.status, 'needsUpdate');
  assert.equal(restoreCheck({}, 'rev', web)?.status, 'needsUpdate');
});
