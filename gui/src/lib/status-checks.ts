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
