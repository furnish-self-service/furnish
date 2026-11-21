import { test } from 'node:test';
import assert from 'node:assert/strict';
import { actionUrl, linkActions, parseActionLink } from './action-link.ts';

test('all six actions round-trip local names as data, including literal query delimiters', () => {
  for (const action of linkActions) for (const kind of ['app', 'kit'] as const) {
    const link = { action, kind, collection: 'Démo + Office', id: 'tools & tips#1' };
    assert.deepEqual(parseActionLink(actionUrl(link)), link);
  }
});
test('rejects ambiguous targets, remote sources and malformed links', () => {
  for (const query of ['app=a', 'collection=c&app=', 'collection=c&app=a&kit=k', 'collection=c&app=a&app=b',
    'collection=a&collection=b', 'collection=c&app=a&endpoint=x', 'collection=..&app=x', 'collection=x&app=a%2fb',
    'collection=c&app=%ff', 'collection=c&app=x%00', 'collection=c&app=x%XZ', 'collection=c&app=x#fragment']) {
    assert.equal(parseActionLink('furnish://install?' + query), undefined, query);
  }
  for (const prefix of ['https://install', 'furnish://install/extra', 'furnish://install@other', 'furnish://add', 'furnish://run']) {
    assert.equal(parseActionLink(prefix + '?collection=c&app=a'), undefined);
  }
});
