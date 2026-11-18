import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createHash, generateKeyPairSync, sign } from 'node:crypto';
import { copyFile, mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { promisify } from 'node:util';
import { bytes, readManifest } from '../scripts/update-feed-download.mjs';

async function serve(t, handle) {
  const server = createServer(handle);
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  t.after(() => {
    server.closeAllConnections();
    return new Promise(resolve => server.close(resolve));
  });
  return `http://127.0.0.1:${server.address().port}`;
}

test('waits for the expected version after both an HTTP error and a stale deployment', async t => {
  let reads = 0;
  const url = await serve(t, (_req, res) => {
    reads++;
    if (reads === 1) return res.writeHead(503).end();
    res.end(JSON.stringify({ version: reads === 2 ? 'old' : 'new', deployment: reads }));
  });
  assert.deepEqual(await readManifest(url, 'new', { waitMs: 5_000, retryMs: 1 }), { version: 'new', deployment: 3 });
  assert.equal(reads, 3);
});

test('a stale publication fails with its last observed version', async t => {
  const url = await serve(t, (_req, res) => res.end('{"version":"old"}'));
  await assert.rejects(readManifest(url, 'new', { waitMs: 300, retryMs: 75 }), /propagation timed out.*expected version new, last observed old/);
});

for (const stage of ['headers', 'body']) {
  test(`aborts a download stalled in its ${stage}`, { timeout: 3_000 }, async t => {
    const url = await serve(t, (_req, res) => {
      if (stage === 'body') {
        res.writeHead(200).write('a');
        const timer = setInterval(() => res.write('b'), 10);
        res.on('close', () => clearInterval(timer));
      }
    });
    await assert.rejects(bytes(url, 150), /download timed out after 150 ms/);
  });
}

test('propagation caps even the first request when no manifest arrives', { timeout: 3_000 }, async t => {
  const url = await serve(t, () => {});
  await assert.rejects(readManifest(url, 'new', { waitMs: 200, requestMs: 60_000 }), /propagation timed out.*expected version new, last observed none/);
});

test('the CLI verifies the selected signed feed and fails on stale versions or modified artifacts', async t => {
  const dir = await mkdtemp(join(tmpdir(), 'furnish-feed-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  await mkdir(join(dir, 'scripts'));
  await mkdir(join(dir, 'gui/src-tauri'), { recursive: true });
  for (const name of ['verify-update-feed.mjs', 'update-feed-download.mjs']) {
    await copyFile(new URL(`../scripts/${name}`, import.meta.url), join(dir, 'scripts', name));
  }

  // Give the isolated app its own key. The real CLI always reads its app's embedded public key.
  const { publicKey, privateKey } = generateKeyPairSync('ed25519');
  const id = Buffer.from('fixture1');
  const rawKey = publicKey.export({ type: 'spki', format: 'der' }).subarray(-32);
  const encode = (value) => Buffer.from(value).toString('base64');
  const pubkey = encode(`untrusted comment: fixture\n${encode(Buffer.concat([Buffer.from('Ed'), id, rawKey]))}\n`);
  await writeFile(join(dir, 'gui/src-tauri/tauri.conf.json'), JSON.stringify({ plugins: { updater: { pubkey } } }));
  const artifact = Buffer.from('package');
  const sig = sign(null, createHash('blake2b512').update(artifact).digest(), privateKey);
  const comment = 'version:1.2.3';
  const globalSig = sign(null, Buffer.concat([sig, Buffer.from(comment)]), privateKey);
  const signature = encode(`untrusted comment: fixture\n${encode(Buffer.concat([Buffer.from('ED'), id, sig]))}\ntrusted comment: ${comment}\n${encode(globalSig)}\n`);

  let manifest;
  let downloaded = artifact;
  let feedReads = 0;
  const url = await serve(t, (req, res) => {
    if (req.url === '/latest.json') {
      feedReads++;
      res.end(JSON.stringify(manifest));
    } else res.end(downloaded);
  });
  manifest = { version: '1.2.3', platforms: { 'windows-x86_64': { url: `${url}/app.msi`, signature } } };
  const cli = (feed, expected, folder) => promisify(execFile)(process.execPath, [
    join(dir, 'scripts/verify-update-feed.mjs'), feed, ...(folder ? [folder] : []),
  ], { env: { ...process.env, EXPECTED_VERSION: expected, REQUIRE: 'windows-x86_64' }, timeout: 5_000 });
  const local = join(dir, 'latest.json');
  await writeFile(local, JSON.stringify(manifest));
  await writeFile(join(dir, 'app.msi'), artifact);
  assert.match((await cli(local, '1.2.3', dir)).stdout, /✓ windows-x86_64/);
  assert.match((await cli(`${url}/latest.json`, '1.2.3')).stdout, /Verifying windows-x86_64 for 1\.2\.3: .*app\.msi/);
  assert.equal(feedReads, 1);
  await assert.rejects(cli(local, '2.0.0', dir), /expected version 2\.0\.0, observed 1\.2\.3/);
  downloaded = Buffer.from('altered');
  await assert.rejects(cli(`${url}/latest.json`, '1.2.3'), /the file doesn't match its signature/);
  downloaded = artifact;
  manifest.version = '1.2.4';
  await assert.rejects(cli(`${url}/latest.json`, '1.2.4'), /signed for version 1\.2\.3, the feed says 1\.2\.4/);
});
