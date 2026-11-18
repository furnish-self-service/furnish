import { test } from 'node:test';
import assert from 'node:assert/strict';
import { completedActivity, historyLimit, readHistory, recordProgress, writeHistory, type Run } from './activity-history.ts';

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
  Object.assign(run, { result: 'failed', finishedAt: 20, error: 'private error' });
  const entry = completedActivity('work/vpn', run, [{ level: 'output', message: 'private stdout' }]);
  assert.deepEqual(entry.details?.run.steps, run.steps);
  writeHistory('/first', Array.from({ length: historyLimit + 2 }, (_, i) => ({ ...entry, summary: { ...entry.summary, id: String(i) } })));
  assert.doesNotMatch([...files.values()][0], /private|Furnish\/Command/);
  const restored = readHistory('/first');
  assert.equal(restored.length, historyLimit);
  assert.equal(restored[0].summary.failedStep, 'Configure VPN');
  assert.equal(restored[0].details, undefined);
  assert.deepEqual(readHistory('/second'), []);
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
