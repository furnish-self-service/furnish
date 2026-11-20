// The startup scan and queued operations can ask about the same app concurrently.
// Share the native read until it finishes or the app or catalog changes.
export class StatusChecks {
  private pending = new Map<string, Promise<boolean>>();

  check(key: string, read: (current: () => boolean) => Promise<boolean>): Promise<boolean> {
    const active = this.pending.get(key);
    if (active) return active;
    const current = () => this.pending.get(key) === result;
    const result: Promise<boolean> = Promise.resolve().then(() => read(current)).finally(() => {
      if (current()) this.pending.delete(key);
    });
    this.pending.set(key, result);
    return result;
  }

  invalidate(key: string) {
    this.pending.delete(key);
  }

  clear() {
    this.pending.clear();
  }
}

// Read the current selection between checks. A selection can reorder the outstanding
// keys, but cannot start another check or reinsert a key that was already visited.
export function* scanOrder(keys: string[], focused: () => readonly string[], verified: (key: string) => boolean) {
  const remaining = new Set(keys);
  const catalog = remaining.values();
  let selection: readonly string[] | undefined;
  let position = 0;
  while (remaining.size) {
    const current = focused();
    if (current !== selection) {
      selection = current;
      position = 0;
    }
    let next: string | undefined;
    while (position < current.length && next === undefined) {
      const key = current[position++];
      if (remaining.has(key) && !verified(key)) next = key;
    }
    next ??= catalog.next().value;
    if (next === undefined) return;
    remaining.delete(next);
    yield next;
  }
}
