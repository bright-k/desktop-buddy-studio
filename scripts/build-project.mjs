import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { join, resolve, relative, isAbsolute, basename } from "node:path";
import * as fs from "node:fs/promises";
import { cargoExecutable, toolchainEnvironment } from "./toolchain.mjs";
import { tarExecutable } from "./archive-tools.mjs";

const root = fileURLToPath(new URL("..", import.meta.url));
const args = process.argv.slice(2);
const project = resolve(root, args[0] || process.env.BUDDY_PROJECT_PATH || "examples/typing-cat");
const development = args.includes("--development") || process.env.BUDDY_BUILD_DEVELOPMENT === "1";
if (args.slice(1).some((arg) => arg !== "--development"))
  throw new Error("Usage: node scripts/build-project.mjs [project-directory] [--development]");
const target = process.platform === "darwin" && process.arch === "arm64" ? "macos-arm64"
  : process.platform === "win32" && process.arch === "x64" ? "windows-x64" : null;
if (!target || (process.env.BUDDY_BUILD_TARGET && process.env.BUDDY_BUILD_TARGET !== target))
  throw new Error("Build on Apple Silicon macOS for macos-arm64, or Windows x64 for windows-x64.");
const actualProject = await fs.realpath(project);
const within = relative(await fs.realpath(root), actualProject);
if (within.startsWith("..") || isAbsolute(within))
  throw new Error("Place the project inside this source checkout (for example, projects/com.example.mybuddy).");
function step(executable, parameters, capture = false) {
  const result = spawnSync(executable, parameters, {
    cwd: root, env: toolchainEnvironment(), encoding: "utf8",
    stdio: capture ? ["ignore", "pipe", "inherit"] : "inherit",
  });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`Build step failed (${result.status}): ${executable}`);
  return result.stdout;
}
step(process.execPath, ["scripts/prepare-notices.mjs", "--check"]);
step(process.execPath, ["node_modules/typescript/bin/tsc", "-p", "apps/player/tsconfig.json"]);
step(cargoExecutable(), ["build", "--locked", "-p", "buddy-cli"]);
const cli = join(root, "target", "debug", process.platform === "win32" ? "buddy.exe" : "buddy");
const info = JSON.parse(step(cli, ["validate", actualProject, "--json"], true));
if (process.env.BUDDY_EXPECTED_PROJECT_HASH && info.contentHash !== process.env.BUDDY_EXPECTED_PROJECT_HASH)
  throw new Error("The project content hash differs from the Studio build request.");
const output = join(root, "artifacts", "project-builds", target);
await fs.mkdir(output, { recursive: true });
await fs.writeFile(join(output, "source.json"), JSON.stringify(info, null, 2) + "\n");
const stage = await fs.mkdtemp(join(root, ".buddy-build-"));
try {
  const pack = join(stage, "template");
  step(cli, ["template-pack", "--source", root, "--output", pack]);
  const result = JSON.parse(step(cli, ["build", actualProject, "--template-pack", pack, "--target", target,
    "--output", output, ...(development ? ["--development"] : [])], true));
  // Include the completed app and report; exclude multi-GB Cargo caches and prior builds.
  const archive = join(root, "artifacts", `project-${target}.tar.gz`);
  step(tarExecutable, ["-czf", archive, "-C", output, "source.json", basename(result.directory)]);
  if (process.env.BUDDY_STUDIO_JOB_ID) {
    const bytes = await fs.readFile(archive);
    await fs.writeFile(join(root, "artifacts", `receipt-${target}.json`), JSON.stringify({
      formatVersion: 1, jobId: process.env.BUDDY_STUDIO_JOB_ID, target,
      projectHash: info.contentHash, identifier: info.identifier,
      engineHash: process.env.BUDDY_ENGINE_HASH, templateHash: result.templateHash,
      bytes: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex"),
    }, null, 2) + "\n");
  }
  console.log(JSON.stringify({ ...result, archive }, null, 2));
} finally {
  await fs.rm(stage, { recursive: true, force: true });
}
