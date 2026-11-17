// node --test --experimental-strip-types src/lib/history.test.ts
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createHistory } from './history.ts';

test('back and forward move through the pages like a browser', () => {
  const h = createHistory<{ page: string }>();
  const show = (page: string) => h.seen({ page });
  show('discover');
  assert.equal(h.back(), undefined); // nothing behind the first page
  show('installed');
  show('updates');
  show('updates'); // the same page again adds nothing
  const back = h.back()!;
  assert.equal(back.page, 'installed');
  h.seen(back); // the app shows it: no new entry
  assert.equal(h.back()!.page, 'discover');
  assert.equal(h.back(), undefined);
  assert.equal(h.forward()!.page, 'installed');
  assert.equal(h.forward()!.page, 'updates');
  assert.equal(h.forward(), undefined);
  // A new page after going back drops what was ahead.
  h.back();
  show('settings');
  assert.equal(h.forward(), undefined);
  assert.equal(h.back()!.page, 'installed');
});

test('it keeps at most the given number of pages behind', () => {
  const h = createHistory<number>(String, 3);
  for (const n of [1, 2, 3, 4, 5]) h.seen(n);
  assert.deepEqual([h.back(), h.back(), h.back(), h.back()], [4, 3, 2, undefined]);
});
