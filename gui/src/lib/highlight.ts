// Minimal YAML highlighting for the DSC configuration view. Escapes HTML first, then wraps
// comments, keys, strings and DSC expressions ("[...]") in spans. Not a parser: one line at a time.
const escape = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

function line(raw: string): string {
  const hash = raw.search(/(^|\s)#/);
  const code = hash < 0 ? raw : raw.slice(0, hash);
  const comment = hash < 0 ? '' : `<span class="y-comment">${escape(raw.slice(hash))}</span>`;
  // Strings first: the key step below adds markup containing quotes.
  const html = escape(code)
    .replace(/("[^"]*"|'[^']*')/g, (s) =>
      /^["']\[.*\]["']$/.test(s) ? `<span class="y-expr">${s}</span>` : `<span class="y-string">${s}</span>`,
    )
    // "key:" at line start, optionally after a list dash
    .replace(/^(\s*(?:-\s+)?)([\w$.\-]+)(:)/, '$1<span class="y-key">$2</span>$3');
  return html + comment;
}

export const highlightYaml = (source: string) => source.split('\n').map(line).join('\n');
