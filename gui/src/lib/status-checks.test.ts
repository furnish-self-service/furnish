import { test } from 'node:test';
import assert from 'node:assert/strict';
import { StatusChecks } from './status-checks.ts';

test('a startup scan and a resumed dependent share the live result in either order', async () => {
  for (const verified of [false, true]) {
    const checks = new StatusChecks();
    let reads = 0;
    const read = async (current: () => boolean) => {
      reads++;
      assert.ok(current());
      return verified;
    };
    const first = checks.check('settings', read);
    const second = checks.check('settings', read);
    assert.deepEqual(await Promise.all([first, second]), [verified, verified]);
    assert.equal(reads, 1);
    await checks.check('settings', read);
    assert.equal(reads, 2);
  }
});

test('an operation or catalog change invalidates old reads without removing their replacement', async () => {
  for (const reset of ['app', 'catalog']) {
    const checks = new StatusChecks();
    const oldGate = Promise.withResolvers<void>();
    const newGate = Promise.withResolvers<void>();
    const old = checks.check('app', async current => { await oldGate.promise; return current(); });
    if (reset === 'app') checks.invalidate('app');
    else checks.clear();
    const fresh = checks.check('app', async current => { await newGate.promise; return current(); });
    oldGate.resolve();
    assert.equal(await old, false);
    assert.equal(checks.check('app', async () => assert.fail('duplicate read')), fresh);
    newGate.resolve();
    assert.equal(await fresh, true);
  }
});
