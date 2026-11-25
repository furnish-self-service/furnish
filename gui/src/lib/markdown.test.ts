import assert from 'node:assert/strict';
import test from 'node:test';
import { renderDescription } from './markdown.ts';

test('formatted detail and compact descriptions share inline formatting', () => {
  const source = '## Setup\n\n- **Open** the `client`\n- [Guide](https://example.org)\n\n![Profile](images/a%20b.png)';
  const full = renderDescription(source);
  assert.match(full, /<h2>Setup<\/h2>/);
  assert.match(full, /<ul>/);
  assert.match(full, /<strong>Open<\/strong> the <code>client<\/code>/);
  assert.match(full, /<a href="https:\/\/example.org">Guide<\/a>/);
  assert.match(full, /alt="Profile".*data-collection-image="images\/a b.png"/);
  assert.equal(renderDescription(source, true), 'Setup <strong>Open</strong> the <code>client</code> Guide Profile');
});

test('only Markdown controls markup and only HTTPS URLs become active links or remote images', () => {
  const html = renderDescription('<b>hello</b> ![A & B](https://example.org/p.png) [local](file:///tmp/page) ![share](//example.org/a)');
  assert.match(html, /&lt;b&gt;hello&lt;\/b&gt;/);
  assert.match(html, /alt="A &amp; B"[^>]*src="https:\/\/example.org\/p.png"/);
  assert.doesNotMatch(html, /href=|src="\/\/|data-collection-image=/);
});
