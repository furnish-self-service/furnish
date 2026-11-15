// Translations without a library: a dictionary per language, `{name}` placeholders, plurals by
// `Intl.PluralRules`. The reactive part (the current language) is in i18n.svelte.ts.
import { en } from './locales/en.ts';
import { de } from './locales/de.ts';

/** A text, or one per plural category ("one", "other", …) chosen by the `n` parameter. */
export type Entry = string | { one: string; other: string };
export type Key = keyof typeof en;
export type Lang = 'en' | 'de';
export type Params = Record<string, string | number>;

export const dictionaries: Record<Lang, Record<Key, Entry>> = { en, de };

/** The language for the setting: German for a German system language, else English. */
export function resolve(choice: 'system' | Lang | undefined, systemLanguage: string | undefined): Lang {
  if (choice === 'en' || choice === 'de') return choice;
  return systemLanguage?.slice(0, 2).toLowerCase() === 'de' ? 'de' : 'en';
}

export function translate(lang: Lang, key: Key, params: Params = {}): string {
  const entry = dictionaries[lang][key] ?? en[key];
  const text =
    typeof entry === 'string' ? entry : entry[new Intl.PluralRules(lang).select(Number(params.n)) === 'one' ? 'one' : 'other'];
  return text.replace(/\{(\w+)\}/g, (all, name) => (name in params ? String(params[name]) : all));
}

/** Text with `<b>…</b>` and `<code>…</code>` marks split into parts, so templates can render
 *  those as elements (the texts themselves never become markup). */
export function parts(text: string): { text: string; tag: 'b' | 'code' | null }[] {
  const out: { text: string; tag: 'b' | 'code' | null }[] = [];
  const pieces = text.split(/<(b|code)>(.*?)<\/\1>/);
  // split with two groups: plain, tag, inner, plain, tag, inner, …
  for (let i = 0; i < pieces.length; i += 3) {
    if (pieces[i]) out.push({ text: pieces[i], tag: null });
    if (i + 2 < pieces.length) out.push({ text: pieces[i + 2], tag: pieces[i + 1] as 'b' | 'code' });
  }
  return out;
}

/** `parts` of an entry, then its `{name}` placeholders filled in: marks in the values (an address
 *  from a link, say) stay text. */
export function markedParts(template: string, params: Params): { text: string; tag: 'b' | 'code' | null }[] {
  return parts(template).map((p) => ({ ...p, text: p.text.replace(/\{(\w+)\}/g, (all, name) => (name in params ? String(params[name]) : all)) }));
}

/** The `{name}` placeholders of an entry, sorted, for checking that translations keep them. */
export function placeholders(entry: Entry): string[] {
  const texts = typeof entry === 'string' ? [entry] : Object.values(entry);
  return [...new Set(texts.flatMap((t) => [...t.matchAll(/\{(\w+)\}/g)].map((m) => m[1])))].sort();
}
