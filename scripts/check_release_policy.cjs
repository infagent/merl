// Exercise the pinned release generator without credentials or a GitHub client.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { Manifest, VERSION, setLogger } = require(path.resolve(process.argv[2]));
assert.equal(VERSION, '17.6.0', 'use the version locked by the Release Please action');
setLogger(Object.fromEntries(['info', 'warn', 'error', 'debug', 'trace'].map(key => [key, () => {}])));

const root = path.resolve(__dirname, '..');
const config = fs.readFileSync(path.join(root, 'release-please-config.json'), 'utf8');

async function generatedRelease(previous, message) {
  const files = {
    'release-please-config.json': config,
    '.release-please-manifest.json': JSON.stringify({ '.': previous }),
    'Cargo.toml': `[package]\nname = "synthetic-merl"\nversion = "${previous}"\n`,
    'Cargo.lock': `version = 4\n\n[[package]]\nname = "synthetic-merl"\nversion = "${previous}"\n`,
  };
  // Only read capabilities exist. Unexpected I/O or any mutation fails the test.
  const source = {
    repository: { owner: 'example', repo: 'synthetic-merl', defaultBranch: 'main' },
    async getFileJson(name) { return JSON.parse(files[name]); },
    async getFileContentsOnBranch(name) {
      assert.ok(Object.hasOwn(files, name), `unexpected source read: ${name}`);
      return { parsedContent: files[name] };
    },
    async *releaseIterator() {},
    async *tagIterator() {},
    async *mergeCommitIterator() {
      yield { sha: 'a'.repeat(40), message, files: ['src/main.rs'] };
    },
  };
  const manifest = await Manifest.fromManifest(source, 'main');
  const candidates = await manifest.buildPullRequests();
  assert.equal(candidates.length, 1);
  const proposal = candidates[0];
  const updates = Object.fromEntries(proposal.updates.map(update => [
    update.path, update.updater.updateContent(files[update.path]),
  ]));
  return { proposal, updates };
}

async function main() {
  for (const [previous, message, expected] of [
    ['0.1.0', 'feat!: replace the synthetic protocol', '0.2.0'],
    ['0.1.0', 'feat: add a synthetic command', '0.2.0'],
    ['0.1.0', 'fix: correct a synthetic retry', '0.1.1'],
    ['0.2.0', 'fix!: reject an incompatible synthetic input', '0.3.0'],
    ['1.2.3', 'feat!: replace the synthetic protocol', '2.0.0'],
  ]) {
    const { proposal, updates } = await generatedRelease(previous, message);
    assert.equal(proposal.version.toString(), expected, `${previous}: ${message}`);
    assert.ok(proposal.title.toString().includes(expected));
    assert.ok(proposal.body.toString().includes(expected));
    assert.equal(JSON.parse(updates['.release-please-manifest.json'])['.'], expected);
    for (const file of ['Cargo.toml', 'Cargo.lock']) {
      assert.ok(updates[file].includes(`version = "${expected}"`), `${file}: ${expected}`);
    }
    console.log(`${previous} + ${message} -> ${expected}`);
  }
}

main().catch(error => { console.error(error); process.exitCode = 1; });
