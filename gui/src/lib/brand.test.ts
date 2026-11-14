// node --test --experimental-strip-types src/lib/brand.test.ts
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { accentShades, brandingCss, contrast } from './brand.ts';

const rgb = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)) as [number, number, number];

test('text in the accent color stays readable, whatever the brand color', () => {
  for (const accent of ['#ffd400', '#00ff66', '#0f766e', '#1a1a1a', '#e30613', '#ffffff']) {
    for (const [dark, canvas] of [[false, '#f4f7f7'], [true, '#0d1a1d']] as const) {
      const s = accentShades(accent, dark);
      assert.ok(contrast(rgb(s['accent-ink']), rgb(canvas)) >= 4.5, `${accent} ink on ${canvas}`);
      assert.ok(contrast(rgb(s['on-accent']), rgb(s.accent)) >= 3, `${accent}: text on the accent`);
    }
  }
});

test('each theme only overrides its own colors', () => {
  const css = brandingCss({ name: 'Acme', logo: null, accent: '#e30613', accentDark: null, light: { canvas: '#ffffff' }, dark: {} });
  const [light, dark] = css.split('\n');
  assert.match(light, /not all and \(prefers-color-scheme: dark\).*--canvas: #ffffff;/);
  assert.doesNotMatch(dark, /--canvas/);
  assert.match(dark, /--accent: #/); // derived from the light brand color
});
