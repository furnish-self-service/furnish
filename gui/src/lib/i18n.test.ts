// node --test --experimental-strip-types src/lib/i18n.test.ts
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { dictionaries, knownError, markedParts, parts, placeholders, pluralRules, resolve, translate, warningText, type Lang } from './i18n.ts';

const { en } = dictionaries;
const others = Object.entries(dictionaries).filter(([lang]) => lang !== 'en') as [Lang, typeof en][];

// One test per language, so a translation's problems show under its own name.
for (const [lang, d] of others) {
  test(`${lang}: the English keys, with their placeholders, marks and the plural forms ${lang} needs`, () => {
    assert.deepEqual(Object.keys(d).sort(), Object.keys(en).sort(), 'keys');
    const marks = (e: string | object) => JSON.stringify(e).match(/<\/?(b|code)>/g)?.length ?? 0;
    // The plural categories whole numbers take in this language (Polish: one, few, many, other).
    // `other` is always required: translate falls back to it.
    const rules = pluralRules(lang);
    const needed = [...new Set([...Array.from({ length: 1001 }, (_, n) => rules.select(n)), 'other'])].sort();
    for (const key of Object.keys(en) as (keyof typeof en)[]) {
      assert.deepEqual(placeholders(d[key]), placeholders(en[key]), `placeholders of ${key}`);
      assert.equal(typeof d[key], typeof en[key], `plural or not: ${key}`);
      if (typeof d[key] === 'object') assert.deepEqual(Object.keys(d[key]).sort(), needed, `plural forms of ${key}`);
      const forms = (e: unknown) => (typeof e === 'string' ? [e] : Object.values(e as object));
      const want = marks(typeof en[key] === 'string' ? en[key] : (en[key] as { other: string }).other);
      for (const form of forms(d[key])) assert.equal(marks(form), want, `<b>/<code> marks of ${key}`);
    }
  });
}

test('the system language decides unless the setting names one', () => {
  assert.equal(resolve('system', 'de-AT'), 'de');
  assert.equal(resolve('system', 'de'), 'de');
  assert.equal(resolve('system', 'en-US'), 'en');
  assert.equal(resolve('system', 'fr-FR'), 'fr');
  assert.equal(resolve('system', 'nl-BE'), 'nl');
  assert.equal(resolve('system', 'sv-SE'), 'en');
  assert.equal(resolve(undefined, undefined), 'en');
  assert.equal(resolve('en', 'de-DE'), 'en');
  assert.equal(resolve('de', 'en-US'), 'de');
});

test('parameters and plurals', () => {
  assert.equal(translate('en', 'common.apps', { n: 1 }), '1 app');
  assert.equal(translate('en', 'common.apps', { n: 2 }), '2 apps');
  assert.equal(translate('de', 'common.apps', { n: 0 }), '0 Apps');
  assert.equal(translate('de', 'kit.install', { n: 1 }), '1 App installieren');
  assert.equal(translate('pl', 'common.apps', { n: 2 }), '2 aplikacje');
  assert.equal(translate('pl', 'common.apps', { n: 12 }), '12 aplikacji');
  assert.equal(translate('pt', 'common.apps', { n: 0 }), '0 apps');
  assert.equal(translate('de', 'nav.home', { brand: 'Acme Apps' }), 'Acme Apps, zu Entdecken wechseln');
});

test('marks in filled-in values stay text', () => {
  assert.deepEqual(markedParts('add <b>{origin}</b>?', { origin: 'x</b><code>y</code>' }), [
    { text: 'add ', tag: null },
    { text: 'x</b><code>y</code>', tag: 'b' },
    { text: '?', tag: null },
  ]);
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

test('ignored settings in the app language, with the key marked as code', () => {
  const endpoint = { code: 'endpoint', host: 'furnish.acme.tld', key: 'reporting', reason: 'unknown variant `x`' } as const;
  assert.equal(translate('en', ...warningText(endpoint)), '<code>reporting</code> from furnish.acme.tld: unknown variant `x`');
  assert.equal(translate('de', ...warningText(endpoint)), '<code>reporting</code> von furnish.acme.tld: unknown variant `x`');
  assert.equal(
    translate('fr', ...warningText({ code: 'unknownSetting', key: 'osSpecific.macos.x' })),
    '<code>osSpecific.macos.x</code>, défini par votre organisation : ce paramètre n’existe pas',
  );
  // Every code the backend sends has a text in every language.
  for (const code of ['endpoint', 'policy', 'user', 'unknownSetting', 'invalidSettings']) {
    for (const d of Object.values(dictionaries)) assert.ok(`settings.warning.${code}` in d, code);
  }
});

// The backend's texts, as src/nixos_apps.rs tests them (service_only), inside a resource's failure.
test('the reasons furnish gives for not changing the NixOS configuration are translated', () => {
  const failure = (text: string) => `Command: Resource 'furnish-nixos.dsc.resource.sh' [exit code 1] furnish: ${text}`;
  const notPreset =
    "only the furnish service changes the NixOS configuration, and it installs only the apps of preset collections. " +
    "The collection 'mine' isn't preset: ask your administrator to preset it (services.furnish.policy.collections.preset) " +
    "or to list its furnish address in the policy (services.furnish.policy.endpoints), and, unless you're an administrator, " +
    "to set services.furnish.policy.service.allowedGroup to a group you're in";
  const off = 'only the furnish service changes the NixOS configuration: ask your administrator to turn it on (services.furnish.service.enable)';
  const perUser = 'only the furnish service changes the NixOS configuration, and acme-ssh installs for each person (perUser), so furnish runs its steps as you';
  assert.equal(knownError('en', failure(notPreset)), failure(notPreset));
  assert.equal(knownError('de', failure(notPreset)), failure(translate('de', 'error.nixos.notPreset', { collection: 'mine' })));
  assert.match(knownError('de', notPreset), /Die Sammlung 'mine' ist nicht vorgegeben/);
  assert.equal(knownError('fr', failure(off)), failure(translate('fr', 'error.nixos.serviceOff')));
  assert.equal(knownError('nl', perUser), translate('nl', 'error.nixos.perUser', { app: 'acme-ssh' }));
  assert.equal(knownError('de', 'something else'), 'something else');
});

test('DSC-wrapped close-and-retry errors preserve the named applications', () => {
  const prefix = "Command: Resource '/usr/local/bin/furnish-gui' [exit code 2] furnish: ";
  const names = "Acme's Desktop, Cache Helper";
  const backend = `${prefix}Close ${names}, then retry this change\n`;
  const translated = knownError('de', backend);
  assert.equal(translated, prefix + translate('de', 'error.file.requiresClosed', { names }) + '\n');
  assert.ok(translated.includes(names) && translated !== backend);
});
