import { test } from 'node:test';
import assert from 'node:assert/strict';
import { defaultDependencies, planInstall, releaseDependents, type Queued } from './install-queue.ts';

const dependency = (key: string, required = true, selected = required) => ({ key, required, selected });
const app = (key: string, ...dependencies: ReturnType<typeof dependency>[]) => ({ key, dependencies });
const job = (key: string, ...prerequisites: string[]): Queued => ({ key, prerequisites, action: 'install', trigger: 'user' });

test('a shared prerequisite is installed once and both dependents wait for it', () => {
  const brew = app('brew');
  const left = app('left', dependency('brew'));
  const right = app('right', dependency('brew'));
  const tools = app('tools', dependency('left'), dependency('right'));
  const plan = planInstall(tools, defaultDependencies(tools), [brew, left, right, tools], [], () => false);
  assert.equal(plan.error, undefined);
  assert.deepEqual(plan.installs, [
    { key: 'brew', prerequisites: [] },
    { key: 'left', prerequisites: ['brew'] },
    { key: 'right', prerequisites: ['brew'] },
    { key: 'tools', prerequisites: ['left', 'right'] },
  ]);
  const queue = [job('left', 'brew'), job('right', 'brew'), job('tools', 'left', 'right'), job('unrelated')];
  assert.equal(queue.find(q => !q.prerequisites.length)?.key, 'unrelated', 'failed or cancelled brew holds only its dependents');
  releaseDependents(queue, 'brew'); // whether retried from Activity or installed directly
  assert.deepEqual(queue.map(q => q.prerequisites), [[], [], ['left', 'right'], []]);
  releaseDependents(queue, 'left');
  assert.deepEqual(queue[2].prerequisites, ['right']);
});

test('a queued or running install is joined with its existing prerequisites and choices', () => {
  const tools = app('tools', dependency('brew'));
  const plan = planInstall(tools, [], [tools, app('brew', dependency('default'))], [job('brew', 'chosen')], () => false);
  assert.deepEqual(plan.installs, [{ key: 'tools', prerequisites: ['brew'] }]);
  assert.deepEqual([...plan.members], ['chosen', 'brew', 'tools']);
});

test('required and selected dependencies are respected, satisfied items need no install', () => {
  const tools = app('tools', dependency('required', true, false), dependency('already'),
    dependency('extra', false), dependency('unchecked', false, true));
  const plan = planInstall(tools, ['extra'], [tools, app('required'), app('extra')], [], key => key === 'already');
  assert.equal(plan.error, undefined);
  assert.deepEqual(plan.installs.at(-1), { key: 'tools', prerequisites: ['required', 'extra'] });
  assert.deepEqual(plan.installs.map(q => q.key), ['required', 'extra', 'tools']);
});

test('cycles, including a failed prerequisite waiting on its own retry, cannot enter the queue', () => {
  const a = app('a', dependency('b'));
  const b = app('b', dependency('a'));
  for (const pending of [[], [job('b', 'a')]]) {
    const plan = planInstall(a, ['b'], [a, b], pending, () => false);
    assert.deepEqual(plan.error, { kind: 'cycle', detail: 'a → b → a' });
    assert.deepEqual(plan.installs, []);
  }
  const missing = planInstall(a, ['b'], [a], [], () => false);
  assert.deepEqual(missing.error, { kind: 'missing', detail: 'b' });
  assert.deepEqual(missing.installs, []);
});

test('a new dependent joins a failing branch, and only a requested retry schedules the prerequisite', () => {
  const brew = app('brew', dependency('setup'));
  const tools = app('extra', dependency('brew'));
  const pending = [job('first', 'brew')];
  const catalog = [brew, tools, app('setup')];
  const join = planInstall(tools, ['brew'], catalog, pending, () => false);
  assert.deepEqual(join.installs, [{ key: 'extra', prerequisites: ['brew'] }]);
  assert.deepEqual([...join.members], ['brew', 'extra']);
  const retry = planInstall(brew, ['setup'], catalog, pending, () => false);
  assert.deepEqual(retry.installs, [{ key: 'setup', prerequisites: [] }, { key: 'brew', prerequisites: ['setup'] }]);
});

test('an uninstall cannot silently satisfy a root install or one of its prerequisites', () => {
  const brew = app('brew');
  const tools = app('tools', dependency('brew'));
  const pending: Queued[] = [{ ...job('brew'), action: 'uninstall' }];
  for (const root of [brew, tools]) {
    const plan = planInstall(root, defaultDependencies(root), [tools, brew], pending, () => true);
    assert.deepEqual(plan.error, { kind: 'uninstalling', detail: 'brew' });
    assert.deepEqual(plan.installs, []);
  }
});
