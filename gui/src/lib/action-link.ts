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
