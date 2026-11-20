import { test } from 'node:test';
import assert from 'node:assert/strict';
import { summarizeKit } from './kit-status.ts';
import type { Item, Status } from './api';

const item = (key: string, required = true): Item => ({ key, required, selected: required });

test('kit completion and progress use the applicable members and distinguish optional extras', () => {
  const items = [item('linux'), item('windows'), item('mac'), item('extra', false)];
  const states: Record<string, Status> = { linux: 'needsUpdate', windows: 'unavailable', mac: 'unavailable', extra: 'notInstalled' };
  const summary = () => summarizeKit(items, key => states[key]);
  assert.deepEqual(summary(), { installed: 1, applicable: 2, unavailable: 2, complete: true });
  states.extra = 'installed';
  assert.deepEqual(summary(), { installed: 2, applicable: 2, unavailable: 2, complete: true });
  items.push(item('unknown'));
  assert.equal(summary().complete, false);
  for (const status of ['unavailable', 'notInstalled', undefined] as const) {
    assert.equal(summarizeKit([item('only', false)], () => status).complete, false);
  }
});
