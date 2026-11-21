import { test } from 'node:test';
import assert from 'node:assert/strict';
import { actionUrl, formatActionLink, linkActions, parseActionLink } from './action-link.ts';

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

test('share formats preserve one action target and treat the label as text', () => {
  const url = actionUrl({ action: 'update', collection: 'work', kind: 'app', id: 'vpn)' });
  const label = 'Update [VPN](x) <b>&amp;';
  assert.equal(formatActionLink(url, 'url', label), url);
  assert.equal(formatActionLink(url, 'markdown', label), String.raw`[Update \[VPN\]\(x\) \<b\>\&amp\;](furnish://update?collection=work&app=vpn%29)`);
  const html = formatActionLink(url, 'html', label);
  assert.ok(html.startsWith('<a href="furnish://update?collection=work&amp;app=vpn%29" style="'));
  assert.ok(html.endsWith('">Update [VPN](x) &lt;b&gt;&amp;amp;</a>'));
});
