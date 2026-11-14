// Plain-language descriptions of common DSC conditions; anything else is shown as written.
const family = String.raw`context\(\)\.os\.family`;
const only = new RegExp(String.raw`^\[equals\(${family}, '(\w+)'\)\]$`);
const not = new RegExp(String.raw`^\[not\(equals\(${family}, '(\w+)'\)\)\]$`);

export function describeCondition(condition: string): string | null {
  const o = condition.match(only);
  if (o) return `Only on ${o[1]}`;
  const n = condition.match(not);
  if (n) return `Everywhere except ${n[1]}`;
  return null;
}
