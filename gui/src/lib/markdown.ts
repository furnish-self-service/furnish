import MarkdownIt from 'markdown-it';

const md = new MarkdownIt({ html: false });
const escape = md.utils.escapeHtml;
const https = (url: string) => /^https:\/\//i.test(url);

md.renderer.rules.link_open = (tokens, index, _, env) => {
  const href = String(tokens[index].attrGet('href') ?? '');
  if (env?.compact) return '';
  return https(href) ? `<a href="${escape(href)}">` : '<a>';
};
md.renderer.rules.link_close = (_, __, ___, env) => env?.compact ? '' : '</a>';
md.renderer.rules.image = (tokens, index, options, env, renderer) => {
  const image = tokens[index];
  const alt = escape(renderer.renderInlineAsText(image.children ?? [], options, env));
  if (env?.compact) return alt;
  const src = String(image.attrGet('src') ?? '');
  const attrs = `alt="${alt}" loading="lazy" referrerpolicy="no-referrer"`;
  if (https(src)) return `<img ${attrs} src="${escape(src)}">`;
  if (!src || /^[a-z][a-z0-9+.-]*:|^[\/\\]/i.test(src)) return alt;
  try {
    return `<img ${attrs} data-collection-image="${escape(decodeURIComponent(src))}">`;
  } catch {
    return alt;
  }
};

export function renderDescription(source: string, compact = false): string {
  if (!compact) return md.render(source, { compact });
  // Cards and control labels keep inline formatting without nested controls or large media.
  const env = { compact };
  return md.parse(source, env).map((token) => {
    if (token.type === 'inline') return md.renderer.renderInline(token.children ?? [], md.options, env);
    if (token.type === 'fence' || token.type === 'code_block') return `<code>${escape(token.content.trimEnd())}</code>`;
    return '';
  }).filter(Boolean).join(' ');
}
