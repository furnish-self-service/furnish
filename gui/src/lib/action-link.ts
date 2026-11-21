export const linkActions = ['install', 'update', 'uninstall'] as const;
export type LinkAction = typeof linkActions[number];
export interface ActionLink {
  action: LinkAction;
  kind: 'app' | 'kit';
  collection: string;
  id: string;
}

export function actionUrl({ action, kind, collection, id }: ActionLink) {
  return `furnish://${action}?${new URLSearchParams({ collection, [kind]: id })}`;
}

export function parseActionLink(link: string): ActionLink | undefined {
  const match = /^furnish:\/\/(install|update|uninstall)\?([^#\s]+)$/u.exec(link);
  if (!match || /%(?![\da-f]{2})/i.test(match[2])) return;
  const params = new URLSearchParams(match[2]);
  const kind = params.has('app') ? 'app' : 'kit';
  const collection = params.get('collection');
  const id = params.get(kind);
  const valid = (s: string | null): s is string => !!s && s.trim() === s && s !== '.' && s !== '..'
    && !/[\/\\\p{Cc}\p{Cf}\uFFFD]/u.test(s);
  if (params.size !== 2 || !valid(collection) || !valid(id)) return;
  return { action: match[1] as LinkAction, kind, collection, id };
}

/** Format a URL produced by actionUrl for pasting into a message or document. */
export function formatActionLink(url: string, format: 'url' | 'html' | 'markdown', label: string) {
  if (format === 'url') return url;
  if (format === 'markdown') {
    // CommonMark supports backslash escapes for ASCII punctuation.
    const plain = label.replace(/\s+/gu, ' ').replace(/[!-/:-@\[-`{-~]/g, '\\$&');
    return `[${plain}](${url})`;
  }
  const escape = (text: string) => text.replaceAll('&', '&amp;').replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;').replaceAll('"', '&quot;');
  const style = 'display:inline-block;padding:8px 16px;border-radius:6px;background:#17585c;color:#fff;font-weight:600;text-decoration:none';
  return `<a href="${escape(url)}" style="${style}">${escape(label)}</a>`;
}
