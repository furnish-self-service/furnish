// Check an update feed (latest.json, see packaging/repo/update-feed.sh) the way furnish's
// updater does: each platform's download matches its signature under the public key the desktop
// app embeds (gui/src-tauri/tauri.conf.json), and the signature names the feed's version.
// Independent of furnish's and Tauri's code: minisign checked with Node's own crypto.
//
//   node scripts/verify-update-feed.mjs <latest.json: file or URL> [folder with the downloads]
//
// Without a folder it downloads each file from its URL; with one it takes the file of the same
// name from anywhere inside it. REQUIRE="windows-x86_64 darwin-aarch64" also fails when the feed
// lacks one of these. EXPECTED_VERSION requires that exact feed version and waits up to five
// minutes for an HTTP deployment to propagate. Exits with 1 on any problem.
import { createHash, createPublicKey, verify } from 'node:crypto';
import { readdirSync, readFileSync } from 'node:fs';
import { basename, join } from 'node:path';
import { bytes, readManifest } from './update-feed-download.mjs';

const [feed, folder] = process.argv.slice(2);
if (!feed) {
  console.error('usage: verify-update-feed.mjs <latest.json file or URL> [folder]');
  process.exit(2);
}
const conf = JSON.parse(readFileSync(new URL('../gui/src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
const lines = (b64) => Buffer.from(b64, 'base64').toString('utf8').trim().split('\n');

// minisign public key: "Ed" + key id (8 bytes) + Ed25519 key (32 bytes).
const pub = Buffer.from(lines(conf.plugins.updater.pubkey)[1], 'base64');
if (pub.subarray(0, 2).toString() !== 'Ed') throw new Error('not a minisign public key');
const keyId = pub.subarray(2, 10);
const key = createPublicKey({ key: Buffer.concat([Buffer.from('302a300506032b6570032100', 'hex'), pub.subarray(10)]), format: 'der', type: 'spki' });

const semver = (v) => v.replace(/^v/, '').replace('+alpha.', '-alpha.');

/** minisign signature (base64 of its text): "ED" + key id + signature over BLAKE2b-512 of the
 *  file, then the trusted comment and a signature over both. */
function check(data, signature, version) {
  const [, sigLine, commentLine, globalLine] = lines(signature);
  const sig = Buffer.from(sigLine, 'base64');
  const algorithm = sig.subarray(0, 2).toString();
  if (!sig.subarray(2, 10).equals(keyId)) throw new Error('signed with another key');
  const message = algorithm === 'ED' ? createHash('blake2b512').update(data).digest() : algorithm === 'Ed' ? data : null;
  if (!message) throw new Error(`unknown signature algorithm ${algorithm}`);
  if (!verify(null, message, key, sig.subarray(10, 74))) throw new Error("the file doesn't match its signature");
  const comment = commentLine.replace(/^trusted comment: /, '');
  if (!verify(null, Buffer.concat([sig.subarray(10, 74), Buffer.from(comment)]), key, Buffer.from(globalLine, 'base64')))
    throw new Error('the trusted comment was changed');
  const signed = comment.split('\t').find((f) => f.startsWith('version:'))?.slice('version:'.length);
  if (!signed || semver(signed) !== semver(version)) throw new Error(`signed for version ${signed}, the feed says ${version}`);
}

const manifest = await readManifest(feed, process.env.EXPECTED_VERSION);
console.log(`${feed}: version ${manifest.version}`);
const platforms = Object.entries(manifest.platforms ?? {});
if (!manifest.version || !platforms.length) throw new Error(`${feed}: no version or no platforms`);
let failed = false;
for (const target of (process.env.REQUIRE ?? '').split(/\s+/).filter(Boolean)) {
  if (!manifest.platforms[target]) {
    console.error(`✗ ${target}: not in the feed`);
    failed = true;
  }
}
const local = folder ? readdirSync(folder, { recursive: true }).map(String) : [];
const find = (url) => {
  const name = decodeURIComponent(basename(new URL(url).pathname));
  const hit = local.find((f) => basename(f) === name);
  if (!hit) throw new Error(`no ${name} in ${folder}`);
  return join(folder, hit);
};
for (const [target, { url, signature }] of platforms) {
  try {
    const artifact = folder ? find(url) : url;
    console.log(`Verifying ${target} for ${manifest.version}: ${artifact}`);
    check(await bytes(artifact, 120_000), signature, manifest.version);
    console.log(`✓ ${target}: ${url}`);
  } catch (e) {
    console.error(`✗ ${target}: ${url}: ${e.message}`);
    failed = true;
  }
}
process.exit(failed ? 1 : 0);
