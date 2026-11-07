import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { resolve, join } from "node:path";
import { existsSync, readFileSync, mkdirSync, copyFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { cargoExecutable, toolchainEnvironment } from "./toolchain.mjs";
import { createStudio } from "../studio/server.mjs";
import { run } from "../studio/cli.mjs";
const root = fileURLToPath(new URL("..", import.meta.url));
function step(command, args) {
  const r = spawnSync(command, args, {
    cwd: root,
    env: toolchainEnvironment(),
    stdio: "inherit",
  });
  if (r.status !== 0) process.exit(r.status ?? 1);
}
step(process.execPath, ["scripts/prepare-notices.mjs", "--check"]);
step(cargoExecutable(), [
  "run",
  "--locked",
  "-q",
  "-p",
  "buddy-cli",
  "--",
  "export-contracts",
]);
step(process.execPath, [
  "node_modules/typescript/bin/tsc",
  "-p",
  "apps/studio/tsconfig.json",
]);
step(process.execPath, [
  "node_modules/vite/bin/vite.js",
  "build",
  "--config",
  "apps/studio/vite.config.ts",
]);
// Export may have changed contracts that are part of the trusted template.
// Recompile before freezing the CLI so its embedded manifest matches them.
step(cargoExecutable(), ["build", "--locked", "-q", "-p", "buddy-cli"]);
const compiledCli = join(
  root,
  "target",
  "debug",
  process.platform === "win32" ? "buddy.exe" : "buddy",
);
const hash = createHash("sha256")
    .update(readFileSync(compiledCli))
    .digest("hex")
    .slice(0, 20),
  engineDirectory = join(root, ".buddy", "studio-engines", hash),
  templatePack = join(engineDirectory, "template");
mkdirSync(engineDirectory, { recursive: true });
const cli = join(
  engineDirectory,
  process.platform === "win32" ? "buddy.exe" : "buddy",
);
if (!existsSync(cli)) copyFileSync(compiledCli, cli);
if (!existsSync(join(templatePack, "pack-manifest.json")))
  await run(cli, ["template-pack", "--source", root, "--output", templatePack]);
const doctor = JSON.parse(
  await run(cli, ["doctor"]).catch((e) => e.stdout || "{}"),
);
const development = !doctor.host?.meetsSupportBaseline;
const options = {
  cli,
  templatePack,
  dataRoot: resolve(process.env.BUDDY_STUDIO_DATA || join(root, ".data")),
  port: Number(process.env.BUDDY_STUDIO_PORT || 4340),
  development,
};
let studio;
try {
  studio = await createStudio(options);
} catch (error) {
  if (error.existingUrl) {
    console.log(`이미 실행 중인 Studio: ${error.existingUrl}`);
    if (process.env.BUDDY_STUDIO_OPEN !== "0" && process.platform === "darwin")
      spawnSync("/usr/bin/open", [error.existingUrl]);
    process.exit(0);
  }
  if (error.code !== "EADDRINUSE" || process.env.BUDDY_STUDIO_PORT) throw error;
  studio = await createStudio({ ...options, port: 0 });
}
console.log(
  `\nDesktop Buddy Studio\n${studio.url}\n프로젝트 저장: ${studio.store.root}\n종료: Ctrl+C\n`,
);
if (process.env.BUDDY_STUDIO_OPEN !== "0" && process.platform === "darwin")
  spawnSync("/usr/bin/open", [studio.url]);
let closing = false;
for (const signal of ["SIGINT", "SIGTERM"])
  process.on(signal, async () => {
    if (closing) return;
    closing = true;
    await studio.close();
    process.exit(0);
  });
