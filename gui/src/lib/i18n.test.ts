// node --test --experimental-strip-types src/lib/i18n.test.ts
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dictionaries, parts, placeholders, resolve, translate } from './i18n.ts';

const { en, de } = dictionaries;

test('both languages have the same keys', () => {
  assert.deepEqual(Object.keys(de).sort(), Object.keys(en).sort());
});

test('translations keep the placeholders, plural forms and marks of the English text', () => {
  const marks = (e: string | object) => JSON.stringify(e).match(/<\/?(b|code)>/g)?.length ?? 0;
  for (const key of Object.keys(en) as (keyof typeof en)[]) {
    assert.deepEqual(placeholders(de[key]), placeholders(en[key]), `placeholders of ${key}`);
    assert.equal(typeof de[key], typeof en[key], `plural forms of ${key}`);
    assert.equal(marks(de[key]), marks(en[key]), `<b>/<code> marks of ${key}`);
  }
});

test('the system language decides unless the setting names one', () => {
  assert.equal(resolve('system', 'de-AT'), 'de');
  assert.equal(resolve('system', 'de'), 'de');
  assert.equal(resolve('system', 'en-US'), 'en');
  assert.equal(resolve('system', 'fr-FR'), 'en');
  assert.equal(resolve(undefined, undefined), 'en');
  assert.equal(resolve('en', 'de-DE'), 'en');
  assert.equal(resolve('de', 'en-US'), 'de');
});

test('parameters and plurals', () => {
  assert.equal(translate('en', 'common.apps', { n: 1 }), '1 app');
  assert.equal(translate('en', 'common.apps', { n: 2 }), '2 apps');
  assert.equal(translate('de', 'common.apps', { n: 0 }), '0 Apps');
  assert.equal(translate('de', 'kit.install', { n: 1 }), '1 App installieren');
  assert.equal(translate('de', 'nav.home', { brand: 'Acme Apps' }), 'Acme Apps, zu Entdecken wechseln');
});

test('marked text splits into parts without becoming markup', () => {
  assert.deepEqual(parts('a <b>x</b> b <code>y</code>'), [
    { text: 'a ', tag: null },
    { text: 'x', tag: 'b' },
    { text: ' b ', tag: null },
    { text: 'y', tag: 'code' },
  ]);
  assert.deepEqual(parts('<b>only</b>'), [{ text: 'only', tag: 'b' }]);
});
