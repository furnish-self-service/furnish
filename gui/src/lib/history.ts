// Back and forward through the pages visited, like a browser: the mouse's thumb buttons,
// Alt+Left/Right and Cmd+[ / Cmd+]. furnish keeps its page in `store.view`, not in the
// webview's history, so it keeps its own.

/** `seen` gets every page shown; `back` and `forward` return the page to show, or nothing. */
export function createHistory<T>(key: (v: T) => string = (v) => JSON.stringify(v), max = 100) {
  const behind: T[] = [];
  const ahead: T[] = [];
  let current: T | undefined;
  return {
    seen(v: T) {
      if (current !== undefined && key(current) === key(v)) return; // the same page, or a step back/forward
      if (current !== undefined) {
        behind.push(current);
        if (behind.length > max) behind.shift();
      }
      ahead.length = 0;
      current = v;
    },
    previous(): T | undefined { return behind.at(-1); },
    back(): T | undefined {
      const v = behind.pop();
      if (v !== undefined && current !== undefined) ahead.push(current);
      if (v !== undefined) current = v;
      return v;
    },
    forward(): T | undefined {
      const v = ahead.pop();
      if (v !== undefined && current !== undefined) behind.push(current);
      if (v !== undefined) current = v;
      return v;
    },
  };
}
