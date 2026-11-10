import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { basename, delimiter, dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const destination = join(root, 'packaging/notices');
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const inputs = ['Cargo.lock', 'package-lock.json', 'LICENSE', 'packaging/licenses/about.toml', 'packaging/licenses/clarifications.json', 'scripts/prepare-notices.mjs'];
const inputHashes = () => Object.fromEntries(inputs.map(path => [path, sha(readFileSync(join(root, path)))]));
function verifyPrepared() {
  const manifest = JSON.parse(readFileSync(join(destination, 'manifest.json'), 'utf8'));
  if (manifest.formatVersion !== 1 || JSON.stringify(manifest.inputHashes) !== JSON.stringify(inputHashes())) throw new Error('NOTICES_STALE: regenerate notices for the current dependency locks');
  for (const file of manifest.files) {
    if (!/^[a-zA-Z0-9_./-]+$/.test(file.path) || file.path.split('/').includes('..')) throw new Error('Invalid notices path');
    const data = readFileSync(join(destination, file.path));
    if (data.length !== file.bytes || sha(data) !== file.sha256) throw new Error(`NOTICES_INTEGRITY: ${file.path}`);
  }
  process.stdout.write(`Verified notices: ${manifest.crateCount} Rust packages, ${manifest.npmCount} web/tool packages, ${manifest.sourceArchiveCount} source archives\n`);
}
if (process.argv.includes('--check')) {
  verifyPrepared();
} else {
  const cargoBin = join(homedir(), '.cargo/bin');
  const about = join(cargoBin, process.platform === 'win32' ? 'cargo-about.exe' : 'cargo-about');
  const env = { ...process.env, PATH: cargoBin + delimiter + process.env.PATH };
  const scratch = mkdtempSync(join(tmpdir(), 'buddy-notices-'));
  const stage = mkdtempSync(join(root, 'packaging/.notices-'));
  chmodSync(stage, 0o755);
  const write = (path, data) => { mkdirSync(dirname(join(stage, path)), { recursive: true }); writeFileSync(join(stage, path), data); };
  try {
    const version = spawnSync(about, ['--version'], { env, encoding: 'utf8' });
    if (version.status !== 0 || !version.stdout.trim().endsWith('0.9.1')) throw new Error('Maintainer tool required: cargo install cargo-about --version 0.9.1 --locked --features cli');
    const generated = join(scratch, 'cargo.json');
    const result = spawnSync(about, ['generate', '--locked', '--workspace', '--all-features', '--fail', '--format', 'json', '--config', 'packaging/licenses/about.toml', '--output-file', generated], { cwd: root, env, stdio: 'inherit' });
    if (result.status !== 0) throw new Error('cargo-about failed');
    const raw = JSON.parse(readFileSync(generated, 'utf8'));
    const clarifications = JSON.parse(readFileSync(join(root, 'packaging/licenses/clarifications.json'), 'utf8')).packages;
    const packages = new Map();
    const terms = new Map();
    const addTerm = (id, text, source) => {
      if (!text.trim()) throw new Error('Empty license text');
      const key = sha(Buffer.from(id + '\n' + text));
      if (!terms.has(key)) { terms.set(key, { id, path: `terms/${key}.txt`, source }); write(`terms/${key}.txt`, text); }
      return key;
    };
    const packageFor = p => {
      const key = `cargo:${p.name}@${p.version}`;
      if (!packages.has(key)) packages.set(key, { id: key, name: p.name, version: p.version, ecosystem: 'cargo', declaredLicense: p.license, authors: p.authors, repository: p.repository, sourceUrl: `https://crates.io/api/v1/crates/${p.name}/${p.version}/download`, terms: [], supplements: [], sourceArchive: null });
      return packages.get(key);
    };
    const needsSource = new Set();
    for (const license of raw.licenses) {
      for (const used of license.used_by) {
        const p = used.crate;
        if (!p.source) continue; // The complete engine MIT notice is included separately.
        const entry = packageFor(p);
        let text = license.text;
        let source = 'published-crate';
        if (license.source_path === null && license.id !== 'CC0-1.0') {
          const clarification = clarifications.find(c => c.name === p.name && c.version === p.version && c.license === license.id);
          if (!clarification) throw new Error(`NOTICES_MISSING_UPSTREAM: ${entry.id} ${license.id}`);
          const supplement = readFileSync(join(root, 'packaging/licenses', clarification.file));
          if (sha(supplement) !== clarification.sha256) throw new Error(`Changed upstream notice: ${entry.id}`);
          if (!clarification.kind || clarification.kind === 'upstream-license-text') {
            text = supplement.toString('utf8'); source = clarification.source;
          } else {
            // Some upstreams declare MIT by reference; others carry additional
            // terms. Preserve their declaration, exact notice and original source.
            entry.supplements.push(addTerm(clarification.kind, supplement.toString('utf8'), clarification.source));
            needsSource.add(entry.id);
          }
        }
        entry.terms.push(addTerm(license.id, text, source));
        if (license.id === 'MPL-2.0') needsSource.add(entry.id);
      }
    }
    const sourcePackages = raw.crates.filter(c => c.package.source);
    if (packages.size !== sourcePackages.length) throw new Error('Not every resolved Rust package has a license notice');
    const lock = readFileSync(join(root, 'Cargo.lock'), 'utf8');
    for (const { package: p } of sourcePackages) {
      const key = `cargo:${p.name}@${p.version}`;
      if (!needsSource.has(key)) continue;
      const block = lock.split('[[package]]').find(part => part.includes(`name = "${p.name}"\n`) && part.includes(`version = "${p.version}"\n`));
      const expected = block?.match(/\nchecksum = "([0-9a-f]{64})"/)?.[1];
      if (!expected) throw new Error(`Missing locked source checksum: ${key}`);
      const crateRoot = dirname(p.manifest_path);
      const archive = join(dirname(dirname(dirname(crateRoot))), 'cache', basename(dirname(crateRoot)), `${p.name}-${p.version}.crate`);
      let data;
      if (existsSync(archive)) data = readFileSync(archive);
      else {
        const response = await fetch(`https://static.crates.io/crates/${p.name}/${p.name}-${p.version}.crate`);
        if (!response.ok) throw new Error(`Source download failed: ${key}`);
        data = Buffer.from(await response.arrayBuffer());
      }
      if (sha(data) !== expected) throw new Error(`Source checksum mismatch: ${key}`);
      const path = `sources/${p.name}-${p.version}.crate`;
      write(path, data); packages.get(key).sourceArchive = path;
    }
    const npmLock = JSON.parse(readFileSync(join(root, 'package-lock.json'), 'utf8'));
    for (const name of ['@tauri-apps/api', '@tauri-apps/cli', 'vite', 'rollup']) {
      const directory = join(root, 'node_modules', name);
      const p = JSON.parse(readFileSync(join(directory, 'package.json'), 'utf8'));
      if (p.version !== npmLock.packages[`node_modules/${name}`]?.version) throw new Error(`Installed package differs from lock: ${name}`);
      const files = readdirSync(directory).filter(file => /^(licen[cs]e|copying|notice)/i.test(file) && !file.endsWith('.spdx') && statSync(join(directory, file)).isFile()).sort();
      if (!files.length) throw new Error(`Missing web/tool license: ${name}`);
      const entry = { id: `npm:${name}@${p.version}`, name, version: p.version, ecosystem: 'npm', declaredLicense: p.license, authors: p.author ?? p.contributors ?? [], repository: p.repository?.url ?? p.repository ?? null, sourceUrl: `https://www.npmjs.com/package/${name}/v/${p.version}`, terms: [], supplements: [], sourceArchive: null };
      for (const file of files) entry.terms.push(addTerm(p.license ?? 'upstream', readFileSync(join(directory, file), 'utf8'), `npm:${name}/${file}`));
      packages.set(entry.id, entry);
    }
    for (const file of ['LICENSE-Tauri.txt', 'LICENSE-NSIS.txt']) {
      const text = readFileSync(join(root, 'packaging/windows', file), 'utf8');
      addTerm(file, text, `packaging/windows/${file}`);
    }
    write('ENGINE-LICENSE.txt', readFileSync(join(root, 'LICENSE')));
    const inventory = { formatVersion: 1, scope: 'Both supported targets, Rust workspace including build dependencies, and web/bundling tools. This is an attribution inventory, not a runtime dependency list.', packages: [...packages.values()].sort((a, b) => a.id.localeCompare(b.id)), terms: Object.fromEntries([...terms].sort(([a], [b]) => a.localeCompare(b))) };
    write('inventory.json', JSON.stringify(inventory, null, 2) + '\n');
    let plain = 'THIRD-PARTY NOTICES\n\nSee inventory.json for package versions, declarations, authors, exact notice sources and included source archives. Engine terms are in ENGINE-LICENSE.txt. Packages with upstream MIT declarations by reference retain the standard MIT terms plus their exact upstream declaration and source archive. Additional upstream notices are preserved.\n\n';
    for (const entry of inventory.packages) plain += `${entry.id}\nDeclared: ${entry.declaredLicense}\nSource: ${entry.sourceUrl}\nAuthors: ${JSON.stringify(entry.authors)}\nLicense texts: ${entry.terms.join(', ')}\nAdditional notices: ${entry.supplements.join(', ')}\n${entry.sourceArchive ? `Included source: ${entry.sourceArchive}\n` : ''}\n`;
    for (const [key, term] of Object.entries(inventory.terms)) plain += `\n========== ${key} / ${term.id} ==========\nSource: ${term.source}\n\n${readFileSync(join(stage, term.path), 'utf8')}\n`;
    write('THIRD_PARTY_NOTICES.txt', plain);
    const files = [];
    const collect = directory => {
      for (const name of readdirSync(directory).sort()) {
        const path = join(directory, name);
        if (statSync(path).isDirectory()) collect(path);
        else { const data = readFileSync(path); files.push({ path: relative(stage, path).replaceAll('\\', '/'), bytes: data.length, sha256: sha(data) }); }
      }
    };
    collect(stage);
    write('manifest.json', JSON.stringify({ formatVersion: 1, generator: 'cargo-about 0.9.1 + Desktop Buddy notices v1', targets: ['aarch64-apple-darwin', 'x86_64-pc-windows-msvc'], inputHashes: inputHashes(), crateCount: sourcePackages.length, npmCount: 4, sourceArchiveCount: needsSource.size, files }, null, 2) + '\n');
    // A failed generation leaves the previous complete notices untouched.
    const previous = destination + '.previous';
    if (existsSync(previous)) throw new Error('Remove the previous maintainer notices backup before regenerating');
    if (existsSync(destination)) renameSync(destination, previous);
    try { renameSync(stage, destination); } catch (error) { if (existsSync(previous)) renameSync(previous, destination); throw error; }
    rmSync(previous, { recursive: true, force: true });
    verifyPrepared();
  } finally {
    rmSync(stage, { recursive: true, force: true });
    rmSync(scratch, { recursive: true, force: true });
  }
}
