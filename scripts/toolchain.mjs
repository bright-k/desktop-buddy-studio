import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { delimiter, join } from 'node:path';

const definition = readFileSync(new URL('../rust-toolchain.toml', import.meta.url), 'utf8');
export const rustToolchain = definition.match(/^channel\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"\s*$/m)?.[1];
if (!rustToolchain) throw new Error('The bundled Rust toolchain must specify an exact version.');

export function cargoBinDirectory(source = process.env) {
  return join(source.CARGO_HOME || join(homedir(), '.cargo'), 'bin');
}

export function cargoExecutable(source = process.env) {
  const candidate = join(cargoBinDirectory(source), process.platform === 'win32' ? 'cargo.exe' : 'cargo');
  return existsSync(candidate) ? candidate : 'cargo';
}

export function toolchainEnvironment(source = process.env) {
  const env = { ...source, RUSTUP_TOOLCHAIN: rustToolchain, RUSTUP_AUTO_INSTALL: '0' };
  // Windows environment variable names are case-insensitive. Pass one PATH
  // entry so Node cannot select an inherited Path over the configured PATH.
  const pathKey = Object.keys(source).find(key => key.toUpperCase() === 'PATH');
  const pathValue = source[pathKey] || '';
  for (const key of Object.keys(env)) if (key.toUpperCase() === 'PATH') delete env[key];
  env.PATH = cargoBinDirectory(source) + delimiter + pathValue;
  return env;
}
