// The current language and the functions components translate and format with. Changing the
// language re-renders every text that uses `t`, without a restart.
import { markedParts, resolve, translate, type Key, type Lang, type Params } from './i18n.ts';

export { parts } from './i18n.ts';

const current = $state({ lang: resolve('system', navigator.language) as Lang });
document.documentElement.lang = current.lang;

/** Apply the `language` setting: `system` follows the system language. */
export function setLanguage(choice: 'system' | Lang | undefined) {
  current.lang = resolve(choice, navigator.language);
  document.documentElement.lang = current.lang;
}

export const lang = () => current.lang;

export const t = (key: Key, params?: Params) => translate(current.lang, key, params);

/** A text with `<b>`/`<code>` marks as parts, with the values filled in afterwards, so marks in a
 *  value (e.g. from a link) never count as marks. */
export const tparts = (key: Key, params: Params = {}) =>
  markedParts(translate(current.lang, key, 'n' in params ? { n: params.n } : {}), params);

/** A time of day, e.g. 14:05. */
export const formatTime = (ms: number, seconds = false) =>
  new Date(ms).toLocaleTimeString(current.lang, { hour: '2-digit', minute: '2-digit', ...(seconds && { second: '2-digit' }) });

/** A day such as 2026-11-02, written the local way; anything else stays as it is. */
export function formatDate(day: string) {
  const m = day.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!m) return day;
  return new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])).toLocaleDateString(current.lang, { dateStyle: 'medium' });
}

/** A duration: milliseconds below a second, else seconds with one decimal. */
export const formatDuration = (ms: number) =>
  ms < 1000
    ? `${ms} ms`
    : `${(ms / 1000).toLocaleString(current.lang, { minimumFractionDigits: 1, maximumFractionDigits: 1 })} s`;
