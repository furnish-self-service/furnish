import { test } from 'node:test';
import assert from 'node:assert/strict';
import { completedActivity, groupProgress, historyLimit, interruptedDependents, isGroup, readHistory, readWaitingQueue, recordProgress, trimHistory, writeHistory, type ActivityGroup, type Run } from './activity-history.ts';

const operation = (): Run => ({ id: 'r', name: 'VPN', trigger: 'autoInstall', kind: 'reinstall', startedAt: 0, steps: [], phaseStart: 0 });
const plan = (name: string) => ({
  key: 'work/vpn', event: 'planned' as const,
  steps: [{ name, type: 'Furnish/Command', action: 'change' as const, details: ['private command'], changes: [] }],
});
const finished = (error: string | null) => ({
  key: 'work/vpn', event: 'finished' as const, index: 0, atMs: 20, durationMs: 5,
  outcome: { action: 'change' as const, changes: [], error, cancelled: false },
});

test('reinstall and fallback keep earlier steps when the engine restarts its step indices', () => {
  const run = operation();
  const first = plan('Remove old config');
  first.steps.push(...plan('Finish old phase').steps);
  recordProgress(run, first);
  recordProgress(run, finished(null));
  recordProgress(run, plan('Apply new config'));
  recordProgress(run, { key: 'work/vpn', event: 'started', index: 0, atMs: 15 });
  assert.deepEqual(run.steps.map(s => s.state), ['done', 'notRun', 'running']);
  recordProgress(run, finished('private failure details'));
  run.result = 'failed';
  run.finishedAt = 20;
  const { summary } = completedActivity('work/vpn', run, []);
  assert.equal(summary.failedStep, 'Apply new config');
  assert.equal(summary.trigger, 'autoInstall');
  assert.deepEqual(run.steps.map(s => s.state), ['done', 'notRun', 'failed']);
});

test('recent history is bounded and profile-specific, with only summary fields surviving restart', () => {
  const files = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => files.get(key) ?? null,
    setItem: (key: string, raw: string) => files.set(key, raw),
  } });
  const run = operation();
  recordProgress(run, plan('Configure VPN'));
  recordProgress(run, finished('private failure details'));
  Object.assign(run, { result: 'failed', finishedAt: 20, error: 'private error', collectionRevision: '123abcd' });
  run.steps[0].step.changes.push({ property: 'password', before: 'private old', after: 'private new' });
  run.steps[0].step.notes = [{ code: 'private', params: { value: 'private option' } }];
  const entry = completedActivity('work/vpn', run, [{ level: 'output', message: 'private stdout' }]);
  assert.deepEqual(entry.details?.run.steps, run.steps);
  writeHistory('/first', Array.from({ length: historyLimit + 2 }, (_, i) => ({ ...entry, summary: { ...entry.summary, id: String(i) } })));
  assert.doesNotMatch([...files.values()][0], /private/);
  const restored = readHistory('/first');
  assert.equal(restored.length, historyLimit);
  assert.ok(!isGroup(restored[0]));
  assert.equal(restored[0].summary.failedStep, 'Configure VPN');
  assert.equal(restored[0].summary.collectionRevision, '123abcd');
  assert.deepEqual(restored[0].summary.steps, [{ name: 'Configure VPN', type: 'Furnish/Command', action: 'change', state: 'failed', outcome: 'change', durationMs: 5 }]);
  assert.equal(restored[0].summary.failure, 'step');
  assert.equal(restored[0].details, undefined);
  assert.deepEqual(readHistory('/second'), []);
  const oldStep = { name: 'Legacy VPN step', state: 'done' };
  files.set([...files.keys()][0], JSON.stringify([{ ...entry.summary, steps: [oldStep] }]));
  const [legacy] = readHistory('/first');
  assert.ok(!isGroup(legacy));
  assert.deepEqual(legacy.summary.steps, [oldStep]);
  files.set([...files.keys()][0], '{');
  assert.deepEqual(readHistory('/first'), []);
});

test('a command failure before any step and a cancelled operation preserve their actual outcome', () => {
  const run = operation();
  Object.assign(run, { result: 'failed', finishedAt: 10 });
  assert.equal(completedActivity('work/vpn', run, []).summary.result, 'failed');
  assert.equal(completedActivity('work/vpn', run, []).summary.failedStep, null);
  run.result = 'cancelled';
  assert.equal(completedActivity('work/vpn', run, []).summary.result, 'cancelled');
});

test('kit summaries retain earlier attempts and mark unfinished members interrupted after restart', () => {
  const files = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: (key: string) => files.get(key) ?? null,
    setItem: (key: string, raw: string) => files.set(key, raw),
  } });
  const run = operation();
  recordProgress(run, plan('Configure VPN'));
  recordProgress(run, finished('private failure details'));
  Object.assign(run, { result: 'failed', finishedAt: 20 });
  const attempt = completedActivity('work/vpn', run, [{ level: 'output', message: 'private stdout' }]);
  const kit: ActivityGroup = {
    kind: 'kit', id: 'one-request', key: 'work/tools', name: 'Tools', updatedAt: 21, items: [
      { key: 'work/editor', name: 'Editor', selected: true, state: 'done', attempts: [] },
      { key: 'work/vpn', name: 'VPN', selected: true, state: 'running', runId: 'retry', ownsJob: true, attempts: [attempt] },
      { key: 'work/helper', name: 'Helper', selected: false, state: 'queued', waitingFor: ['work/vpn'], attempts: [] },
    ],
  };
  writeHistory('p', [kit, attempt]);
  const raw = [...files.values()][0];
  assert.doesNotMatch(raw, /private|phaseStart|runId|ownsJob/);
  const [restored, standalone] = readHistory('p');
  assert.ok(isGroup(restored));
  assert.equal(restored.id, kit.id);
  assert.deepEqual(restored.items.map(i => i.state), ['done', 'interrupted', 'interrupted']);
  assert.deepEqual(interruptedDependents(restored, 'work/vpn').map(i => i.key), ['work/helper']);
  assert.deepEqual(restored.items.map(i => i.selected), [true, true, false]);
  assert.equal(restored.items[1].attempts[0].summary.failedStep, 'Configure VPN');
  assert.equal(restored.items[1].attempts[0].details, undefined);
  assert.ok(!isGroup(standalone));
  assert.equal(standalone.summary.id, run.id);
});

test('active kit requests remain tracked while newer unrelated activity exceeds the history limit', () => {
  const run = operation();
  Object.assign(run, { result: 'done', finishedAt: 20 });
  const recent = Array.from({ length: historyLimit + 2 }, () => completedActivity('work/app', run, []));
  const kit: ActivityGroup = {
    kind: 'kit', id: 'active', key: 'work/kit', name: 'Kit', updatedAt: 0,
    items: [{ key: 'work/app', name: 'App', selected: true, state: 'queued', attempts: [] }],
  };
  assert.equal(trimHistory([...recent, kit]).at(-1), kit);
  kit.items[0].state = 'done';
  assert.equal(trimHistory([...recent, kit]).includes(kit), false);
});

test('a restored retry follows the saved prerequisite edges and preserves cancelled and unrelated branches', () => {
  const item = (key: string, state: ActivityGroup['items'][number]['state'], waitingFor: string[] = []) =>
    ({ key, name: key, selected: true, state, waitingFor, attempts: [] });
  const group: ActivityGroup = {
    kind: 'kit', id: 'kit', key: 'work/kit', name: 'Kit', updatedAt: 0, items: [
      item('app', 'interrupted', ['tool']), item('other', 'interrupted'),
      item('tool', 'interrupted', ['bootstrap']), item('bootstrap', 'failed'),
      item('cancelled', 'cancelled', ['bootstrap']), item('done', 'done', ['bootstrap']),
      item('unrelated child', 'interrupted', ['cancelled']),
    ],
  };
  assert.deepEqual(interruptedDependents(group, 'bootstrap').map(i => i.key), ['tool', 'app']);
});

test('update groups retain their identity, skipped updates and redacted package failure on restart', () => {
  let raw = '';
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    getItem: () => raw, setItem: (_key: string, value: string) => { raw = value; },
  } });
  const run = operation();
  recordProgress(run, plan('Install VPN'));
  run.steps[0].step.type = 'Furnish/Package';
  recordProgress(run, finished('sensitive script output and arguments'));
  Object.assign(run, { result: 'failed', finishedAt: 12, collectionRevision: 'sensitive non-revision' });
  const attempt = completedActivity('work/vpn', run, []);
  writeHistory('work', [{
    kind: 'updates', id: 'updates', updatedAt: 3, items: [
      { key: 'work/vpn', name: 'VPN', selected: true, state: 'failed', attempts: [attempt] },
      { key: 'work/old', name: 'Old', selected: true, state: 'skipped', attempts: [] },
    ],
  }]);
  assert.doesNotMatch(raw, /sensitive|script output|arguments/);
  const [group] = readHistory('work');
  assert.ok(isGroup(group));
  assert.equal(group.kind, 'updates');
  assert.equal(group.items[1].state, 'skipped');
  assert.equal(group.items[0].attempts[0].summary.failure, 'package');
  assert.equal(group.items[0].attempts[0].summary.collectionRevision, undefined);
  // Existing versions persisted only these fields, with no per-step history.
  const { failure, steps, action, commit, collectionRevision, ...legacy } = attempt.summary;
  raw = JSON.stringify([legacy, { kind: 'kit', key: 'work/kit', name: 'Kit', id: 'kit', updatedAt: 2,
    items: [{ key: 'work/vpn', selected: true, name: 'VPN', state: 'failed', attempts: [legacy] }] }]);
  const [old, kit] = readHistory('work');
  assert.ok(!isGroup(old) && old.summary.failedStep === 'Install VPN');
  assert.ok(isGroup(kit) && kit.kind === 'kit' && kit.items[0].attempts.length === 1);
});

test('update progress follows failed prerequisite chains and excludes fallbacks and skips', () => {
  const member = (key: string, state: ActivityGroup['items'][number]['state'], waitingFor: string[] = []) =>
    ({ key, name: key, state, waitingFor, selected: true, attempts: [] });
  const group: ActivityGroup = { kind: 'updates', id: 'req', updatedAt: 7, items: [
    member('fallback', 'fallback'), member('skipped', 'skipped'), member('ok', 'done'),
    member('work', 'running'), member('tool', 'failed'), member('middle', 'queued', ['tool']),
    member('app', 'queued', ['middle']), member('later', 'queued', ['work']),
  ] };
  const progress = groupProgress(group, [], key => key === 'work' || key === 'middle');
  assert.equal(progress.total, 8);
  assert.equal(progress.done, 1);
  assert.equal(progress.waiting, 1);
  assert.deepEqual([...progress.blocked], [['middle', ['tool']], ['app', ['tool']]]);
  group.items[4].state = 'running';
  const retry = groupProgress(group, [], key => ['work', 'tool', 'middle'].includes(key));
  assert.equal(retry.blocked.size, 0);
  assert.equal(retry.waiting, 3);
});

test('native revision changes at fallback, including a failure before it can plan a step', () => {
  const run = operation();
  recordProgress(run, { key: 'work/vpn', event: 'revision', revision: 'a123456' });
  recordProgress(run, plan('Original step'));
  recordProgress(run, finished('permission denied: private path'));
  Object.assign(run, { result: 'failed', finishedAt: 5 });
  const first = completedActivity('work/vpn', run, []).summary;
  assert.equal(first.failure, 'permission');
  assert.equal(first.collectionRevision, 'a123456');
  recordProgress(run, { key: 'work/vpn', event: 'revision', revision: 'b123456' });
  const fallback = completedActivity('work/vpn', run, []).summary;
  assert.equal(fallback.collectionRevision, 'b123456');
  assert.equal(fallback.failedStep, null);
  assert.equal(fallback.failure, 'operation');
});


test('a paused dependency branch survives restart, while independent queued work stays interrupted', () => {
  let saved = '';
  Object.defineProperty(globalThis, 'localStorage', { configurable: true, value: {
    setItem: (_: string, value: string) => { saved = value; }, getItem: () => saved,
  }});
  const job = (key: string, prerequisites: string[]) => ({key, action: 'install' as const, trigger: 'user' as const, prerequisites});
  const queue = [
    {...job('fixture/helper', []), waitingForClose: ['Notes Desktop']},
    {...job('fixture/root', ['fixture/helper']), updateOnly: true, chosen: ['fixture/helper']},
    job('fixture/unrelated', []),
  ];
  const group: ActivityGroup = {kind:'updates', id:'batch', updatedAt:1, items:
    queue.map(j => ({key:j.key, name:j.key, selected:true, state:'queued', ownsJob:true, attempts:[]}))};
  writeHistory('work', [group], queue);
  const restored = readWaitingQueue('work');
  assert.deepEqual(restored.map(j => j.key), ['fixture/helper', 'fixture/root']);
  assert.deepEqual(restored[0].waitingForClose, ['Notes Desktop']);
  assert.deepEqual(restored[1].chosen, ['fixture/helper']);
  assert.equal(restored[1].updateOnly, true);
  const [entry] = readHistory('work', new Set(restored.map(j => j.key)));
  assert.ok(isGroup(entry));
  assert.deepEqual(entry.items.map(i => i.state), ['queued', 'queued', 'interrupted']);
  assert.deepEqual([...groupProgress(entry, restored, key => restored.some(j => j.key === key)).blocked.keys()],
    ['fixture/helper', 'fixture/root']);
  writeHistory('work', [group], []);
  assert.deepEqual(readWaitingQueue('work'), []);
});
