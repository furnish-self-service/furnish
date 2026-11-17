// Required versions (src/required.rs): what a collection accepts for an app, described in the
// app's language, and which apps furnish updates right away because of it.
import { translate, type Lang } from './i18n.ts';

export type Op = '>=' | '>' | '<=' | '<' | '=' | '!=';
/** One entry of `versions`; a fixed version is a range with one '='. */
export type Entry = { kind: 'range'; all: { op: Op; version: string }[] } | { kind: 'regex'; pattern: string };

/** What an app's resource needs and what this computer has (`required::Required`). */
export interface Required {
  resource: string;
  /** The installed version; null: not installed. */
  installed: string | null;
  /** The installed version is acceptable (also true when nothing is installed). */
  acceptable: boolean;
  /** The newest version the package manager offers (installers: the collection's file); null: unknown. */
  offered: string | null;
  /** What installs it, e.g. "winget", "pacman". */
  via: string;
  /** A `Furnish/Installer`: `offered` is the version of the collection's file. */
  installer: boolean;
  /** Installed, but the version can't be read (installed outside furnish, say): it can't be checked and is left as it is. */
  unknown: boolean;
  /** The version furnish installs; null: no acceptable version is offered. */
  target: string | null;
  needs: Entry[];
  /** In English, e.g. "128.0.3 or later; winget offers 128.0.1". */
  summary: string;
}

const opKey = { '>=': 'required.ge', '>': 'required.gt', '<=': 'required.le', '<': 'required.lt', '=': 'required.eq', '!=': 'required.ne' } as const;

const list = (lang: Lang, items: string[], type: 'conjunction' | 'disjunction') =>
  new Intl.ListFormat(lang, { type }).format(items);

/** The acceptable versions, e.g. "128.0.3 or later and earlier than 129, or 115.15.0esr". */
export function describeNeeds(lang: Lang, needs: Entry[]): string {
  const entries = needs.map((e) =>
    e.kind === 'regex'
      ? translate(lang, 'required.regex', { pattern: e.pattern })
      : list(lang, e.all.map((c) => translate(lang, opKey[c.op], { version: c.version })), 'conjunction'),
  );
  return list(lang, entries, 'disjunction');
}

/** What the app needs and what is offered, e.g. "Needs 128.0.3 or later; winget offers 128.0.1". */
export function describeRequired(lang: Lang, r: Required): string {
  const needs = describeNeeds(lang, r.needs);
  if (r.installer)
    return r.offered === null
      ? translate(lang, 'required.fileUnknown', { needs })
      : translate(lang, 'required.fileHas', { needs, offered: r.offered });
  return r.offered === null
    ? translate(lang, 'required.needsNone', { needs, via: r.via })
    : translate(lang, 'required.needs', { needs, via: r.via, offered: r.offered });
}

/** The installed version isn't acceptable and an acceptable one is offered: a security update
 *  the collection requires. */
export const urgent = (r: Required | null | undefined): r is Required & { target: string } =>
  !!r && !r.acceptable && r.installed !== null && r.target !== null;

/** No acceptable version is offered, and the app isn't installed at an acceptable one. */
export const unmet = (r: Required | null | undefined): r is Required =>
  !!r && !r.unknown && r.target === null && (r.installed === null || !r.acceptable);

/** Whether to install a required security update right away, and how: ignores the user's
 *  automatic-update choices, declined updates, load and the interval, but never queues an app
 *  twice, tries once per round of status checks (`tried`: a failed or refused attempt waits for
 *  the next check), and leaves apps the system's configuration installs alone. A held app (earlier
 *  version after a failed install) goes to the collection's current version. */
export function urgentAction(
  app: { held: unknown },
  r: Required | null | undefined,
  state: { pending: boolean; declared: boolean; tried: boolean },
): 'install' | 'installCurrent' | null {
  if (!urgent(r) || state.pending || state.declared || state.tried) return null;
  return app.held ? 'installCurrent' : 'install';
}
