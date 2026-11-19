import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { DesktopUpdateStatus } from './api';
import { desktopRestart, type RestartState } from './desktop-restart.ts';

const replacement = { version: '0.1.3', revision: '6a9bce' };
const changed: DesktopUpdateStatus = { changed: true, installed: replacement };
const unchanged: DesktopUpdateStatus = { changed: false, installed: null };

function offered(): RestartState {
  return { installedDesktop: replacement, desktopRestart: 'idle' };
}

test('periodic and focus checks share a probe and never restart the app', async () => {
  const state = offered();
  state.installedDesktop = null;
  let checks = 0;
  const update = desktopRestart(state, () => true, async () => { checks++; return changed; }, async () => assert.fail());
  await Promise.all([update.refresh(), update.refresh()]);
  assert.equal(checks, 1);
  assert.deepEqual(state.installedDesktop, replacement);
});

test('queued work prevents a restart before the click and while rechecking the installed build', async () => {
  const state = offered();
  let idle = false;
  const checked = Promise.withResolvers<DesktopUpdateStatus>();
  let checks = 0;
  const update = desktopRestart(state, () => idle, async () => { checks++; return checked.promise; }, async () => assert.fail());
  assert.equal(await update.restart(), 'busy');
  assert.equal(checks, 0);
  idle = true;
  const clicked = update.restart();
  assert.equal(state.desktopRestart, 'checking');
  idle = false;
  checked.resolve(changed);
  assert.equal(await clicked, 'busy');
  assert.equal(state.desktopRestart, 'idle');
  assert.deepEqual(state.installedDesktop, replacement);
});

test('the clicked probe supersedes a late poll and a same-version revision change needs a new click', async () => {
  const state = offered();
  const poll = Promise.withResolvers<DesktopUpdateStatus>();
  let checks = 0, launches = 0;
  const second = { changed: true, installed: { ...replacement, revision: 'another' } };
  const update = desktopRestart(state, () => true, () => ++checks === 1 ? poll.promise : Promise.resolve(second),
    async expected => { assert.deepEqual(expected, second.installed); launches++; return true; });
  const polling = update.refresh();
  await update.restart();
  poll.resolve(unchanged);
  await polling;
  assert.deepEqual(state.installedDesktop, second.installed);
  assert.equal(launches, 0);
  await update.restart();
  assert.equal(launches, 1);
  assert.equal(state.desktopRestart, 'launching');
});

test('only one launch is requested, native refusals and launch errors release the dispatch gate', async () => {
  const state = offered();
  const launching = Promise.withResolvers<boolean>();
  let launches = 0;
  const update = desktopRestart(state, () => true, async () => changed, async () => {
    launches++;
    if (launches === 2) throw new Error('launch failed');
    return launching.promise;
  });
  const clicked = update.restart();
  await update.restart();
  assert.equal(state.desktopRestart, 'launching');
  await update.refresh();
  await update.restart();
  assert.equal(launches, 1);
  launching.resolve(false);
  assert.equal(await clicked, 'postponed');
  assert.equal(state.desktopRestart, 'idle');
  await assert.rejects(update.restart(), /launch failed/);
  assert.equal(state.desktopRestart, 'idle');
});

test('same installation or missing identity clears the offer before dispatch', async () => {
  for (const status of [unchanged, { changed: false, installed: replacement }, { changed: true, installed: null }]) {
    const state = offered();
    const update = desktopRestart(state, () => true, async () => status, async () => assert.fail());
    assert.equal(await update.restart(), 'unavailable');
    assert.equal(state.installedDesktop, null);
  }
});
