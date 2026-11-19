// Exercise the shipped desktop entry point (including platform wrappers) before any UI starts.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';

const [executable, packageVersion] = process.argv.slice(2);
const output = (program, args) => execFileSync(program, args, { encoding: 'utf8', timeout: 30_000 }).trim();
const revision = process.env.FURNISH_BUILD_REVISION || output('git', ['rev-parse', 'HEAD']);
const version = packageVersion.replace('+alpha.', '-alpha.');
const target = /^host: (.+)$/m.exec(output('rustc', ['-vV']))[1];
const epoch = Number(process.env.SOURCE_DATE_EPOCH ?? output('git', ['show', '-s', '--no-show-signature', '--format=%ct', 'HEAD']));
const sourceDateEpoch = epoch > 1 ? epoch : null;
const actual = JSON.parse(output(executable, ['--build-info']));
assert.deepEqual(actual, { version, revision, target, sourceDateEpoch });
console.log(`${executable} reports ${actual.version} from ${actual.revision} for ${actual.target}`);
