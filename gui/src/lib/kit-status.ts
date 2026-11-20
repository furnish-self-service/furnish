import type { Item, Status } from './api';

export function summarizeKit(items: Item[], statusOf: (key: string) => Status | 'error' | undefined) {
  let installed = 0;
  let unavailable = 0;
  let missingRequired = 0;
  for (const item of items) {
    const status = statusOf(item.key);
    if (status === 'unavailable') unavailable++;
    else if (status === 'installed' || status === 'needsUpdate') installed++;
    else if (item.required) missingRequired++;
  }
  return { installed, applicable: items.length - unavailable, unavailable, complete: installed > 0 && !missingRequired };
}
