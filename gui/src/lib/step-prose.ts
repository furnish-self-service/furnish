// Preserve the original spelling of DSC embedded in a provider's plain-text detail.
export function stepProse(text: string) {
  const parts: { text: string; code: boolean }[] = [];
  const start = /\[+\s*[A-Za-z_]\w*\s*\(/g;
  let from = 0;
  let match: RegExpExecArray | null;
  while ((match = start.exec(text))) {
    let depth = 1;
    let quoted = false;
    let end = start.lastIndex;
    for (; end < text.length && depth; end++) {
      const c = text[end];
      if (c === "'") {
        if (quoted && text[end + 1] === "'") end++;
        else quoted = !quoted;
      } else if (!quoted && c === '[') depth++;
      else if (!quoted && c === ']') depth--;
    }
    if (depth) break;
    start.lastIndex = end;
    if (match[0].startsWith('[[')) continue; // DSC escapes a literal opening bracket by doubling it.
    if (match.index > from) parts.push({ text: text.slice(from, match.index), code: false });
    parts.push({ text: text.slice(match.index, end), code: true });
    from = end;
  }
  if (from < text.length) parts.push({ text: text.slice(from), code: false });
  return parts;
}
