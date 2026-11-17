// node --test --experimental-strip-types src/lib/content.test.ts
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { best, contentLanguage, initial, lookup, matches, normalize, searchTexts, type CollectionTexts } from './content.ts';

const office: CollectionTexts = {
  language: 'en',
  locales: {
    de: {
      categories: { Office: 'Büro' },
      apps: { mail: { name: 'Post', description: 'E-Mail, Kalender und Kontakte' } },
    },
    fr: { apps: { mail: { description: 'Courriel' }, other: { description: 'Autre' } } },
  },
};
const desc = (c: CollectionTexts, wanted: string[], id: string) => lookup(c, wanted, (f) => f.apps?.[id]?.description);

test('matching takes the exact tag, then the primary subtag', () => {
  const available = ['de', 'pt', 'pt-BR'];
  assert.equal(best('pt-BR', available), 'pt-BR');
  assert.equal(best('pt-PT', available), 'pt');
  assert.equal(best('de-AT', available), 'de');
  assert.equal(best('pt', ['pt-PT', 'pt-BR']), 'pt-BR');
  assert.equal(best('fr', available), undefined);
});

test("the app's language, then the fallback, then the collection's own texts", () => {
  assert.equal(desc(office, ['de', 'en'], 'mail'), 'E-Mail, Kalender und Kontakte');
  assert.equal(desc(office, ['de', 'fr'], 'other'), 'Autre');
  assert.equal(desc(office, ['de', 'en'], 'other'), undefined);
  // English is the collection's own language: its texts, before French.
  assert.equal(desc(office, ['en', 'fr'], 'mail'), undefined);
  assert.equal(desc(office, ['it', 'pl'], 'mail'), undefined);
  // A German collection: its own texts for German users, and as the last resort.
  const german: CollectionTexts = { language: 'de', locales: { en: { apps: { mail: { description: 'Email' } } } } };
  assert.equal(desc(german, ['de-AT', 'en'], 'mail'), undefined);
  assert.equal(desc(german, ['fr', 'en'], 'mail'), 'Email');
  assert.equal(desc(german, ['fr', 'it'], 'mail'), undefined);
});

test('following the system keeps its region when the app uses its language', () => {
  assert.equal(contentLanguage('system', 'pt', 'pt-BR'), 'pt-BR');
  assert.equal(contentLanguage(undefined, 'de', 'de-AT'), 'de-AT');
  assert.equal(contentLanguage('system', 'en', 'sv-SE'), 'en');
  assert.equal(contentLanguage('pt', 'pt', 'pt-BR'), 'pt');
});

test('search ignores case and accents', () => {
  assert.equal(normalize('Büro'), 'buro');
  assert.equal(normalize('Ελληνικά Café'), 'ελληνικα cafe');
  assert.ok(matches('buro', ['Büro']));
  assert.ok(matches('BÜRO', ['buro']));
  assert.ok(matches('  ', ['anything']));
  assert.ok(!matches('xyz', ['Büro', null, undefined]));
});

test("search finds texts in the app's language, the fallback, the collection's own, ids and categories", () => {
  const mail = { id: 'mail', name: null, description: 'Email, calendar and contacts', collection: 'office', categories: ['Office'] };
  const find = (q: string, wanted: string[]) => matches(q, searchTexts('apps', mail, office, wanted));
  assert.ok(find('buro', ['de', 'en']), 'translated category');
  assert.ok(find('office', ['de', 'en']), 'category key');
  assert.ok(find('kalender', ['de', 'en']), 'translated description');
  assert.ok(find('post', ['de', 'en']), 'translated name');
  assert.ok(find('calendar', ['de', 'en']), "the collection's own text");
  assert.ok(find('courriel', ['de', 'fr']), 'fallback language');
  assert.ok(!find('courriel', ['de', 'en']), 'other languages are not searched');
  assert.ok(find('MAIL', ['de', 'en']), 'id');
});

test('an icon letter is the first character of the shown name', () => {
  assert.equal(initial('Post'), 'P');
  assert.equal(initial('  écrire'), 'É');
  assert.equal(initial('istanbul', 'tr'), 'İ');
  assert.equal(initial('👩‍💻 Dev tools'), '👩‍💻');
  assert.equal(initial(''), '');
});
