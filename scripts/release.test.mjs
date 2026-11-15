import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {mkdtempSync, mkdirSync, writeFileSync, readFileSync, readdirSync, existsSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join, dirname} from 'node:path';
import {spawnSync} from 'node:child_process';
import {test} from 'node:test';

const run = (script, args, cwd) => spawnSync(process.execPath, [join(import.meta.dirname, script), ...args], {cwd, encoding: 'utf8'});
const fixtureFiles = {
  'Cargo.toml': '[package]\nname = "atlas-agentic"\nversion = "0.1.0"\n',
  'Cargo.lock': '[[package]]\nname = "atlas-agentic"\nversion = "0.1.0"\n',
  'desktop/src-tauri/Cargo.toml': '[package]\nname = "atlas-desktop"\nversion = "0.1.0"\n',
  'desktop/src-tauri/Cargo.lock': '[[package]]\nname = "atlas-agentic"\nversion = "0.1.0"\n\n[[package]]\nname = "atlas-desktop"\nversion = "0.1.0"\n',
  'desktop/src-tauri/tauri.conf.json': JSON.stringify({version: '0.1.0'}),
  'desktop/package.json': JSON.stringify({version: '0.1.0'}),
  'desktop/package-lock.json': JSON.stringify({version: '0.1.0', packages: {'': {version: '0.1.0'}}}),
};
function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'atlas-release-'));
  t.after(() => rmSync(root, {recursive: true, force: true}));
  for (const [path, content] of Object.entries(fixtureFiles)) {
    mkdirSync(dirname(join(root, path)), {recursive: true}); writeFileSync(join(root, path), content);
  }
  return root;
}

test('a matching release tag succeeds without modifying version files', t => {
  const root = fixture(t);
  const result = run('check-release-version.mjs', ['v0.1.0'], root);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /0\.1\.0/);
  for (const [path, content] of Object.entries(fixtureFiles)) assert.equal(readFileSync(join(root, path), 'utf8'), content);
});

test('missing, malformed and prerelease tags fail before release', t => {
  const root = fixture(t);
  for (const args of [[], ['0.1.0'], ['v01.1.0'], ['v0.1.0-rc.1'], ['v0.1.0;echo unsafe']]) {
    const result = run('check-release-version.mjs', args, root);
    assert.notEqual(result.status, 0); assert.match(result.stderr, /vX\.Y\.Z/);
  }
});

for (const path of Object.keys(fixtureFiles)) test(`a stale version in ${path} blocks release`, t => {
  const root = fixture(t);
  writeFileSync(join(root, path), fixtureFiles[path].replaceAll('0.1.0', '0.2.0'));
  const result = run('check-release-version.mjs', ['v0.1.0'], root);
  assert.notEqual(result.status, 0); assert.ok(result.stderr.includes(path), result.stderr);
});

test('npm lock root package version and imported Rust lock version are checked', t => {
  const root = fixture(t);
  const lock = JSON.parse(fixtureFiles['desktop/package-lock.json']); lock.packages[''].version = '0.2.0';
  writeFileSync(join(root, 'desktop/package-lock.json'), JSON.stringify(lock));
  assert.notEqual(run('check-release-version.mjs', ['v0.1.0'], root).status, 0);
  writeFileSync(join(root, 'desktop/package-lock.json'), fixtureFiles['desktop/package-lock.json']);
  writeFileSync(join(root, 'desktop/src-tauri/Cargo.lock'), fixtureFiles['desktop/src-tauri/Cargo.lock'].replace('version = "0.1.0"', 'version = "0.2.0"'));
  assert.notEqual(run('check-release-version.mjs', ['v0.1.0'], root).status, 0);
});

test('installer staging preserves bytes and writes a standard SHA-256 sidecar', t => {
  const root = fixture(t); const source = join(root, 'bundle'); const destination = join(root, 'artifacts');
  mkdirSync(source); const installer = 'Atlas_0.1.0_x64-setup.exe'; const bytes = Buffer.from([77, 90, 0, 1, 2, 255]);
  writeFileSync(join(source, installer), bytes); writeFileSync(join(source, 'unrelated.txt'), 'not released');
  const result = run('prepare-windows-release.mjs', [source, destination], root);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(readdirSync(destination).sort(), [installer, `${installer}.sha256`]);
  assert.deepEqual(readFileSync(join(destination, installer)), bytes);
  assert.equal(readFileSync(join(destination, `${installer}.sha256`), 'utf8'), `${createHash('sha256').update(bytes).digest('hex')}  ${installer}\n`);
  assert.notEqual(run('prepare-windows-release.mjs', [source, destination], root).status, 0, 'never overwrite staged bytes');
});

test('missing, empty or ambiguous installers fail instead of publishing incomplete artifacts', t => {
  const root = fixture(t); const source = join(root, 'bundle'); const destination = join(root, 'artifacts'); mkdirSync(source);
  assert.notEqual(run('prepare-windows-release.mjs', [source, destination], root).status, 0);
  writeFileSync(join(source, 'Atlas-setup.exe'), '');
  assert.notEqual(run('prepare-windows-release.mjs', [source, destination], root).status, 0);
  writeFileSync(join(source, 'Atlas-setup.exe'), 'one'); writeFileSync(join(source, 'Other-setup.exe'), 'two');
  assert.notEqual(run('prepare-windows-release.mjs', [source, destination], root).status, 0);
  assert.equal(existsSync(destination), false);
});
