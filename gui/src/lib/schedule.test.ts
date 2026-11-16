// node --test --experimental-strip-types src/lib/schedule.test.ts
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { serverLooks } from './schedule.ts';

test('furnish looks for collection updates between checks, never with checking set to Never and off at start', () => {
  // Regular checks every 30 minutes or more often apply updates anyway; at start only without updating at start.
  assert.deepEqual(serverLooks(30, true), { atStart: false, everyMinutes: null });
  assert.deepEqual(serverLooks(15, false), { atStart: true, everyMinutes: null });
  // Checks further apart: look in between.
  assert.deepEqual(serverLooks(240, true), { atStart: false, everyMinutes: 30 });
  assert.deepEqual(serverLooks(1440, false), { atStart: true, everyMinutes: 30 });
  // Never, but updating at start: look in between.
  assert.deepEqual(serverLooks(0, true), { atStart: false, everyMinutes: 30 });
  // Never and nothing at start: no network traffic at all.
  assert.deepEqual(serverLooks(0, false), { atStart: false, everyMinutes: null });
});
