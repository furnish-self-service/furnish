// Translations without a library: a dictionary per language, `{name}` placeholders, plurals by
// `Intl.PluralRules`. The reactive part (the current language) is in i18n.svelte.ts.
import { en } from './locales/en.ts';
import { de } from './locales/de.ts';
import { fr } from './locales/fr.ts';
import { es } from './locales/es.ts';
import { it } from './locales/it.ts';
import { pl } from './locales/pl.ts';
import { pt } from './locales/pt.ts';
import { nl } from './locales/nl.ts';

/** A text, or one per plural category the language uses ("one", "few", "many", "other"), chosen
 *  by the `n` parameter; a missing category falls back to "other". */
export type Entry = string | ({ other: string } & Partial<Record<Intl.LDMLPluralRule, string>>);
export type Key = keyof typeof en;
export type Params = Record<string, string | number>;

/** The app's languages, and each one's name in itself (for the language menu). */
export const languages = {
  en: 'English',
  de: 'Deutsch',
  fr: 'Français',
  es: 'Español',
  it: 'Italiano',
  pl: 'Polski',
  pt: 'Português',
  nl: 'Nederlands',
} as const;
export type Lang = keyof typeof languages;

export const dictionaries: Record<Lang, Record<Key, Entry>> = { en, de, fr, es, it, pl, pt, nl };

const isLang = (code: string | undefined): code is Lang => !!code && code in languages;

/** The language for the setting: the system's if furnish has it, else English. */
export function resolve(choice: 'system' | Lang | undefined, systemLanguage: string | undefined): Lang {
  if (choice && choice !== 'system' && isLang(choice)) return choice;
  const system = systemLanguage?.slice(0, 2).toLowerCase();
  return isLang(system) ? system : 'en';
}

export function translate(lang: Lang, key: Key, params: Params = {}): string {
  const entry = dictionaries[lang][key] ?? en[key];
  const text = typeof entry === 'string' ? entry : (entry[pluralRules(lang).select(Number(params.n))] ?? entry.other);
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

// Portugal counts 0 as plural; plain 'pt' follows Brazil, where 0 takes the singular.
export const pluralRules = (lang: Lang) => new Intl.PluralRules(lang === 'pt' ? 'pt-PT' : lang);

/** A value furnish ignored, as the backend reports it (`settings::Warning`): a code and its
 *  fields. `reason` is the check's own message, in English. */
export type SettingsWarning =
  | { code: 'endpoint'; host: string; key: string; reason: string }
  | { code: 'policy' | 'user'; key: string; reason: string }
  | { code: 'unknownSetting'; key: string }
  | { code: 'invalidSettings'; reason: string };

/** The text for an ignored value: its key and parameters, for `translate` or `tparts`. */
export function warningText(w: SettingsWarning): [Key, Params] {
  const { code, ...params } = w;
  return [`settings.warning.${code}`, params];
}

/** Furnish resource errors that DSC includes in a longer diagnostic. Each English entry
 *  matches the backend's text, with placeholders for values such as a process name. */
const knownErrors: Key[] = ['error.nixos.notPreset', 'error.nixos.perUser', 'error.nixos.serviceOff', 'error.file.requiresClosed'];

/** An error text with the known messages in it in the language; the rest stays as it is. */
export function knownError(lang: Lang, text: string): string {
  for (const key of knownErrors) {
    const template = en[key] as string;
    const names = [...template.matchAll(/\{(\w+)\}/g)].map((m) => m[1]);
    const pattern = template
      .split(/\{\w+\}/)
      .map((piece) => piece.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'))
      .join('([^\\r\\n]+?)');
    text = text.replace(new RegExp(pattern, 'g'), (...m) =>
      translate(lang, key, Object.fromEntries(names.map((name, i) => [name, m[i + 1]]))),
    );
  }
  return text;
}
