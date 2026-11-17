// node --test --experimental-strip-types src/lib/required.test.ts
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { describeNeeds, describeRequired, unmet, urgent, urgentAction, type Entry, type Required } from './required.ts';

const range = (...all: [string, string][]) => ({ kind: 'range', all: all.map(([op, version]) => ({ op, version })) }) as Entry;
const required = (r: Partial<Required>): Required => ({
  resource: 'thunderbird',
  installed: '128.0.1',
  acceptable: false,
  offered: '128.0.5',
  via: 'winget',
  installer: false,
  target: '128.0.5',
  needs: [range(['>=', '128.0.3'])],
  summary: '',
  ...r,
});

test('each comparison in words; one range joined with "and", entries with "or"', () => {
  assert.equal(describeNeeds('en', [range(['>=', '128.0.3'])]), '128.0.3 or later');
  assert.equal(describeNeeds('en', [range(['>=', '128.0.3'], ['<', '129'])]), '128.0.3 or later and earlier than 129');
  assert.equal(describeNeeds('en', [range(['>', '1']), range(['<=', '2'])]), 'later than 1 or 2 or earlier');
  assert.equal(describeNeeds('en', [range(['=', '115.15.0esr']), range(['!=', '3'])]), '115.15.0esr or not 3');
  assert.equal(
    describeNeeds('en', [range(['=', '1']), range(['=', '2']), { kind: 'regex', pattern: '^115\\.' }]),
    '1, 2, or a version matching ^115\\.',
  );
  assert.equal(describeNeeds('de', [range(['>=', '128.0.3'], ['<', '129'])]), '128.0.3 oder neuer und älter als 129');
});

test('what the app needs and what the package manager offers, or that it offers none', () => {
  assert.equal(describeRequired('en', required({ offered: '128.0.1', target: null })), 'Needs 128.0.3 or later; winget offers 128.0.1');
  assert.equal(describeRequired('en', required({ offered: null, target: null })), 'Needs 128.0.3 or later; winget offers no version');
  assert.equal(describeRequired('en', required({ installer: true, via: 'installer', offered: '1.2', target: null })), "Needs 128.0.3 or later; the collection's file has 1.2");
  assert.equal(describeRequired('en', required({ installer: true, offered: null, target: null })), "Needs 128.0.3 or later; the version of the collection's file can't be read");
  assert.equal(describeRequired('de', required({ via: 'pacman', offered: '1.0', target: null })), 'Benötigt 128.0.3 oder neuer; pacman bietet 1.0 an');
});

test('a required update is urgent only for an installed version that is not acceptable, with one offered', () => {
  assert.ok(urgent(required({})));
  assert.ok(!urgent(null));
  assert.ok(!urgent(required({ acceptable: true })));
  assert.ok(!urgent(required({ target: null })));
  // Not installed: nothing to update (installing is the user's choice).
  assert.ok(!urgent(required({ installed: null, acceptable: true })));
  assert.ok(unmet(required({ target: null })));
  assert.ok(unmet(required({ installed: null, acceptable: true, target: null })));
  // Installed at an acceptable version: nothing missing even if none is offered.
  assert.ok(!unmet(required({ acceptable: true, target: null })));
  assert.ok(!unmet(required({})));
});

test('required updates install right away, once per check, and a held app goes to the current version', () => {
  const free = { pending: false, declared: false, tried: false };
  assert.equal(urgentAction({ held: null }, required({}), free), 'install');
  assert.equal(urgentAction({ held: { commit: 'abc', date: '2026-11-01' } }, required({}), free), 'installCurrent');
  // Already running or queued (also the one-at-a-time queue: it waits its turn there).
  assert.equal(urgentAction({ held: null }, required({}), { ...free, pending: true }), null);
  // The NixOS or Home Manager configuration installs it: furnish leaves it alone.
  assert.equal(urgentAction({ held: null }, required({}), { ...free, declared: true }), null);
  // Tried in this round of status checks (failed or refused): waits for the next check.
  assert.equal(urgentAction({ held: null }, required({}), { ...free, tried: true }), null);
  assert.equal(urgentAction({ held: null }, required({ target: null }), free), null);
  assert.equal(urgentAction({ held: null }, null, free), null);
});
