import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { dirname, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { cargoExecutable, toolchainEnvironment } from './toolchain.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
let development = false;
let outputArgument;
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--development') development = true;
  else if (args[i] === '--output' && args[i + 1] && !outputArgument) outputArgument = args[++i];
  else throw new Error('Usage: package-cli [--development] [--output directory]');
}
const target = process.platform === 'darwin' && process.arch === 'arm64' ? 'macos-arm64' : process.platform === 'win32' && process.arch === 'x64' ? 'windows-x64' : null;
if (!target) throw new Error('CLI distributions require macOS arm64 or Windows x64');
const version = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version;
const output = resolve(root, outputArgument ?? `artifacts/cli/${target}-${version}${development ? '-development' : ''}`);
if (existsSync(output)) throw new Error(`Output already exists: ${output}`);
mkdirSync(dirname(output), { recursive: true });
const stage = mkdtempSync(join(dirname(output), '.buddy-cli-'));
const cargo = cargoExecutable();
const env = { ...toolchainEnvironment(), CI: 'true', CARGO_TARGET_DIR: join(root, 'target') };
function run(executable, parameters, extra = {}) {
  const result = spawnSync(executable, parameters, { cwd: root, env, stdio: 'inherit', ...extra });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${executable} exited with ${result.status ?? result.signal}`);
  return result;
}
try {
  const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
  run(npm, ['ci', '--ignore-scripts', '--no-audit', '--no-fund'], { shell: process.platform === 'win32' });
  run(npm, ['run', 'build:player'], { shell: process.platform === 'win32' });
  run(process.execPath, [join(root, 'scripts/prepare-notices.mjs'), '--check']);
  run(cargo, ['build', '--release', '--locked', '-p', 'buddy-cli']);
  const binary = process.platform === 'win32' ? 'buddy.exe' : 'buddy';
  cpSync(join(root, 'target/release', binary), join(stage, binary));
  const doctor = run(join(stage, binary), ['doctor'], { stdio: ['ignore', 'pipe', 'inherit'] });
  const environment = JSON.parse(doctor.stdout.toString('utf8'));
  if (!environment.toolchainPolicy.ready) throw new Error(`Build toolchain is not ready: ${environment.toolchainPolicy.problems.join('; ')}`);
  if (!environment.host.meetsSupportBaseline && !development) throw new Error('A supported OS is required. Use --development only for a development distribution.');
  run(join(stage, binary), ['template-pack', '--source', root, '--output', join(stage, 'player-template-pack')]);
  const config = join(stage, 'preview-config.json');
  writeFileSync(config, JSON.stringify({ bundle: { resources: { '../../../packaging/notices/': 'third-party/' }, active: process.platform === 'darwin', macOS: { minimumSystemVersion: development ? '14.0' : '15.0' } } }));
  const tauri = join(root, 'node_modules/@tauri-apps/cli/tauri.js');
  const parameters = [tauri, 'build', '--features', 'preview', '--config', config];
  if (process.platform === 'darwin') parameters.push('--bundles', 'app'); else parameters.push('--no-bundle');
  if (development) parameters.push('--debug');
  parameters.push('--', '--locked');
  run(process.execPath, parameters, { cwd: join(root, 'apps/player'), env });
  rmSync(config);
  const profile = development ? 'debug' : 'release';
  mkdirSync(join(stage, 'player-preview'));
  if (process.platform === 'darwin') {
    cpSync(join(root, `target/${profile}/bundle/macos/Desktop Buddy Preview.app`), join(stage, 'player-preview/Desktop Buddy Preview.app'), { recursive: true });
  } else {
    cpSync(join(root, `target/${profile}/buddy-player.exe`), join(stage, 'player-preview/buddy-player.exe'));
  }
  cpSync(join(root, 'packaging/notices'), join(stage, 'LICENSES'), { recursive: true });
  const notices = JSON.parse(readFileSync(join(root, 'packaging/notices/manifest.json'), 'utf8'));
  writeFileSync(join(stage, 'release-report.json'), JSON.stringify({ formatVersion: 1, version, target, development, templateHash: environment.templateHash, toolchain: environment.tools, toolchainPolicy: environment.toolchainPolicy, host: environment.host, externalDeviceValidation: 'not-performed', previewRuntimeValidation: 'not-performed', thirdPartyNotices: { status: 'bundled-and-verified', rustPackages: notices.crateCount, webToolPackages: notices.npmCount, sourceArchives: notices.sourceArchiveCount } }, null, 2));
  writeFileSync(join(stage, 'README.txt'), `Desktop Buddy CLI ${version} (${target})\n${development ? '개발용 배포 묶음이며 정식 지원 OS 검증을 대신하지 않습니다.' : '최초 MVP 검증용 배포 묶음입니다. 외부 기기 실행 검증 결과는 별도 확인하세요.'}\n\n현재 디렉토리에서:\n  ./buddy doctor\n  ./buddy new my-buddy --name "My Buddy"\n  ./buddy validate my-buddy\n  ./buddy preview my-buddy${development ? ' --development' : ''}\n  ./buddy build my-buddy${development ? ' --development' : ''}\nWindows에서는 ./buddy 대신 .\\buddy.exe를 사용하세요.\n\npreview는 공통 Player를 사용하고 트레이 종료 또는 Ctrl+C로 끝냅니다. 입력 권한은 Player 설정에서 사용자가 요청합니다. 한글 키워드는 두벌식 물리 키 순서로 판정합니다.\n앱 제작에는 Rust/Node/OS 빌드 도구가 필요합니다. 생성 앱 실행에는 개발 도구가 필요 없습니다. player-template-pack과 player-preview를 CLI 옆에 유지하세요. 원래 소스 저장소는 필요하지 않습니다.\n이 묶음의 SHA256SUMS로 파일을 확인할 수 있습니다. 공식 서명·공증은 포함하지 않습니다. 종속성 고지·라이선스와 소스 아카이브는 LICENSES 폴더에 있습니다.\n`);
  const records = [];
  function collect(directory) {
    for (const name of readdirSync(directory).sort()) {
      const path = join(directory, name);
      if (statSync(path).isDirectory()) collect(path);
      else records.push(`${createHash('sha256').update(readFileSync(path)).digest('hex')}  ${relative(stage, path).replaceAll('\\', '/')}`);
    }
  }
  collect(stage);
  writeFileSync(join(stage, 'SHA256SUMS'), records.join('\n') + '\n');
  renameSync(stage, output);
  process.stdout.write(`Created ${output}\n`);
} finally {
  rmSync(stage, { recursive: true, force: true });
}
