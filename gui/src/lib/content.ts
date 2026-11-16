// Collection texts in the user's languages, and search over them. Collections translate their
// texts in `locales/<lang>.yaml`; the same selection runs in Rust for the CLI (src/locale.rs):
// the app's language if the collection has it, else the fallback language, else the collection's
// own texts. A language matches by its exact tag, then by its primary subtag (pt-BR finds pt).

export interface Texts {
  name?: string;
  description?: string;
}
export interface ParameterTexts {
  label?: string;
  description?: string;
  /** Allowed value as text (`true`, `1`, `developer`) → label. */
  allowedValues?: Record<string, string>;
}
export interface LocaleFile {
  categories?: Record<string, string>;
  apps?: Record<string, Texts>;
  kits?: Record<string, Texts>;
  parameters?: Record<string, ParameterTexts>;
}
/** A collection's own language and its translations. */
export interface CollectionTexts {
  language: string;
  locales: Record<string, LocaleFile>;
}

const primary = (tag: string) => tag.split('-')[0].toLowerCase();

/** The best of `available` for `wanted`: the same tag, else the primary subtag alone, else the
 *  first (sorted) with the same primary subtag. Ignores case. */
export function best(wanted: string, available: string[]): string | undefined {
  const sorted = [...available].sort();
  const w = wanted.toLowerCase();
  const p = primary(wanted);
  return (
    sorted.find((a) => a.toLowerCase() === w) ??
    sorted.find((a) => a.toLowerCase() === p) ??
    sorted.find((a) => primary(a) === p)
  );
}

/** The translation to show, trying `wanted` in order; undefined: the collection's own text. A
 *  wanted language the collection's own texts are in ends the search with those. */
export function lookup(
  c: CollectionTexts | undefined,
  wanted: string[],
  get: (f: LocaleFile) => string | undefined,
): string | undefined {
  if (!c) return undefined;
  const available = [...Object.keys(c.locales), c.language];
  for (const w of wanted) {
    const tag = best(w, available);
    if (tag === undefined) continue;
    if (tag.toLowerCase() === c.language.toLowerCase()) return undefined;
    const text = get(c.locales[tag]);
    if (text) return text;
  }
  return undefined;
}

/** The translations that count for the wanted languages (what search looks through). */
export function filesFor(c: CollectionTexts | undefined, wanted: string[]): LocaleFile[] {
  if (!c) return [];
  const available = [...Object.keys(c.locales), c.language];
  return [...new Set(wanted.map((w) => best(w, available)))]
    .filter((tag): tag is string => !!tag && tag.toLowerCase() !== c.language.toLowerCase())
    .map((tag) => c.locales[tag]);
}

/** The language for collection texts: the app's (`ui`); following the system, its own tag
 *  (pt-BR) when the app uses its language. */
export function contentLanguage(choice: string | undefined, ui: string, systemLanguage: string | undefined): string {
  if ((choice ?? 'system') === 'system' && systemLanguage && primary(systemLanguage) === ui) return systemLanguage;
  return ui;
}

/** An allowed value as `allowedValues` keys write it. */
export const valueKey = (value: unknown) => (typeof value === 'string' ? value : JSON.stringify(value));

/** Lower case without accents: "Büro" → "buro". */
export const normalize = (text: string) => text.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();

/** Whether the query is part of one of the texts, ignoring case and accents. */
export function matches(query: string, texts: (string | null | undefined)[]): boolean {
  const q = normalize(query.trim());
  return !q || texts.some((t) => !!t && normalize(t).includes(q));
}

/** What search looks at for an app or a kit: its id, name and description in the wanted
 *  languages and the collection's own, its collection, and its categories (keys and names). */
export function searchTexts(
  kind: 'apps' | 'kits',
  item: { id: string; name?: string | null; description: string; collection: string; categories: string[] },
  c: CollectionTexts | undefined,
  wanted: string[],
): (string | null | undefined)[] {
  const translated = filesFor(c, wanted).flatMap((f) => [
    f[kind]?.[item.id]?.name,
    f[kind]?.[item.id]?.description,
    ...item.categories.map((k) => f.categories?.[k]),
  ]);
  return [item.id, item.name, item.description, item.collection, ...item.categories, ...translated];
}
