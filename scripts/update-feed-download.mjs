// A request's deadline covers headers, redirects and the complete response body.
import { readFileSync } from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';

export async function bytes(where, timeoutMs) {
  if (!/^https?:/.test(where)) return readFileSync(where);
  const signal = AbortSignal.timeout(timeoutMs);
  try {
    const res = await fetch(where, { signal });
    if (!res.ok) {
      await res.body?.cancel();
      throw new Error(`HTTP ${res.status}`);
    }
    return Buffer.from(await res.arrayBuffer());
  } catch (err) {
    const reason = signal.aborted ? `download timed out after ${timeoutMs} ms` : err.cause?.message ?? err.message;
    throw new Error(`${where}: ${reason}`, { cause: err });
  }
}

// Fetch and verify against the same manifest. Pages can still be serving the previous
// deployment. A local manifest or a check without an expected version needs only one read.
export async function readManifest(feed, expected, { requestMs = 20_000, waitMs = 300_000, retryMs = 10_000 } = {}) {
  const poll = Boolean(expected) && /^https?:/.test(feed);
  const deadline = performance.now() + waitMs;
  let observed;
  let lastError;
  console.log(`Reading ${feed}${expected ? `, expecting version ${expected}` : ''}`);
  do {
    const remaining = Math.ceil(deadline - performance.now());
    if (remaining <= 0) break;
    try {
      const manifest = JSON.parse((await bytes(feed, Math.min(requestMs, remaining))).toString('utf8'));
      if (typeof manifest?.version !== 'string' || !manifest.version) throw new Error('missing feed version');
      observed = manifest.version;
      if (!expected || observed === expected) return manifest;
      throw new Error(`expected version ${expected}, observed ${observed}`);
    } catch (err) {
      if (!poll) throw err;
      lastError = err.message;
      console.warn(`Waiting for version ${expected}: ${lastError}`);
    }
    await sleep(Math.max(0, Math.min(retryMs, deadline - performance.now())));
  } while (performance.now() < deadline);
  throw new Error(`${feed}: propagation timed out after ${waitMs} ms; expected version ${expected}, last observed ${observed ?? 'none'}; ${lastError}`);
}
