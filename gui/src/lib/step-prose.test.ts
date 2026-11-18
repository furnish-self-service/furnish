import { test } from 'node:test';
import assert from 'node:assert/strict';
import { stepProse } from './step-prose.ts';

test('keeps nested DSC expressions, quoted brackets and surrounding prose intact', () => {
  const a = "[equals(parameters('teams')[0], 'it''s ] ready')]";
  const b = '[context().os.family]';
  const sentence = `Use ${a} on ${b}. <unknown> [optional]`;
  const parts = stepProse(sentence);
  assert.deepEqual(parts.filter(p => p.code).map(p => p.text), [a, b]);
  assert.equal(parts.map(p => p.text).join(''), sentence);
});

test('leaves ordinary brackets, escaped DSC and incomplete expressions in prose', () => {
  const text = "Read [docs], [[parameters('literal')] and [context().os.family";
  assert.deepEqual(stepProse(text), [{ text, code: false }]);
});
