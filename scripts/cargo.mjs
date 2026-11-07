import { spawnSync } from 'node:child_process';
import { cargoExecutable, toolchainEnvironment } from './toolchain.mjs';
const result = spawnSync(cargoExecutable(), process.argv.slice(2), { stdio: 'inherit', env: toolchainEnvironment() });
if (result.error) console.error(result.error.message);
process.exit(result.status ?? 1);
