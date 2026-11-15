import {readFile} from 'node:fs/promises';

function stringField(table, key) {
  return table.match(new RegExp(`^\\s*${key}\\s*=\\s*"([^"\\r\\n]+)"\\s*(?:#.*)?$`, 'm'))?.[1];
}

async function manifestVersion(path) {
  const text = await readFile(path, 'utf8');
  const table = text.split(/^\s*\[package\]\s*$/m)[1]?.split(/^\s*\[/m)[0];
  return table && stringField(table, 'version');
}

async function lockedVersion(path, name) {
  const text = await readFile(path, 'utf8');
  const packages = text.split(/^\s*\[\[package\]\]\s*$/m)
    .filter(table => stringField(table, 'name') === name);
  return packages.length === 1 ? stringField(packages[0], 'version') : undefined;
}

try {
  const args = process.argv.slice(2);
  const match = args.length === 1 && /^v((?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*))$/.exec(args[0]);
  if (!match) throw new Error('Use a stable version tag vX.Y.Z (for example v0.1.0).');
  const expected = match[1];
  const readJson = async path => JSON.parse(await readFile(path, 'utf8'));
  const lock = await readJson('desktop/package-lock.json');
  const versions = [
    ['desktop/src-tauri/tauri.conf.json', (await readJson('desktop/src-tauri/tauri.conf.json')).version],
    ['desktop/package.json', (await readJson('desktop/package.json')).version],
    ['desktop/package-lock.json', lock.version],
    ['desktop/package-lock.json (root package)', lock.packages?.['']?.version],
    ['Cargo.toml', await manifestVersion('Cargo.toml')],
    ['desktop/src-tauri/Cargo.toml', await manifestVersion('desktop/src-tauri/Cargo.toml')],
    ['Cargo.lock (atlas-agentic)', await lockedVersion('Cargo.lock', 'atlas-agentic')],
    ['desktop/src-tauri/Cargo.lock (atlas-agentic)', await lockedVersion('desktop/src-tauri/Cargo.lock', 'atlas-agentic')],
    ['desktop/src-tauri/Cargo.lock (atlas-desktop)', await lockedVersion('desktop/src-tauri/Cargo.lock', 'atlas-desktop')],
  ];
  const mismatches = versions.filter(([, value]) => value !== expected);
  if (mismatches.length) throw new Error(mismatches.map(([path, value]) => `${path}: expected ${expected}, found ${value ?? 'missing version'}`).join('\n'));
  console.log(`Release version verified: ${expected}`);
} catch (error) {
  console.error(`Release validation failed: ${error.message}`);
  process.exitCode = 1;
}
