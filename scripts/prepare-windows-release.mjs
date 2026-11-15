import {createReadStream, constants} from 'node:fs';
import {readdir, stat, mkdir, copyFile, writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {join, resolve} from 'node:path';

try {
  const args = process.argv.slice(2);
  if (args.length !== 2) throw new Error('Usage: prepare-windows-release.mjs <NSIS directory> <new staging directory>');
  const [source, destination] = args.map(path => resolve(path));
  const installers = (await readdir(source, {withFileTypes: true}))
    .filter(entry => entry.isFile() && entry.name.endsWith('-setup.exe'));
  if (installers.length !== 1) throw new Error(`Expected exactly one NSIS installer, found ${installers.length}`);
  const name = installers[0].name;
  const original = join(source, name);
  if (!(await stat(original)).size) throw new Error('NSIS installer is empty');
  // A fresh directory and exclusive writes prevent stale assets or overwrites.
  await mkdir(destination);
  const staged = join(destination, name);
  await copyFile(original, staged, constants.COPYFILE_EXCL);
  const digest = createHash('sha256');
  for await (const chunk of createReadStream(staged)) digest.update(chunk);
  await writeFile(`${staged}.sha256`, `${digest.digest('hex')}  ${name}\n`, {flag: 'wx'});
  console.log(`Staged Windows installer and SHA-256 checksum: ${name}`);
} catch (error) {
  console.error(`Windows release staging failed: ${error.message}`);
  process.exitCode = 1;
}
