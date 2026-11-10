import * as fs from "node:fs/promises";
import { join } from "node:path";
import { run } from "./cli.mjs";
import { requireId, StudioError } from "./store.mjs";
import { tarExecutable } from "../scripts/archive-tools.mjs";

export async function exportProject(store, id, revision) {
  requireId(revision);
  return store.exclusive(id, async () => {
    const meta = await store.metadata(id);
    if (meta.revision !== revision)
      throw new StudioError("최신 저장 구성으로 다시 내보내세요.", 409);
    const stage = await fs.mkdtemp(join(store.root, ".staging", "export-"));
    try {
      const info = JSON.parse(await run(store.cli, [
        "snapshot", await store.currentPath(id), "--output", join(stage, meta.id),
      ]));
      await fs.writeFile(join(stage, "export.json"), JSON.stringify({
        formatVersion: 1, packageId: meta.id, revision, contentHash: info.contentHash,
      }, null, 2) + "\n");
      await fs.writeFile(join(stage, "BUILD.md"), `# ${meta.id}\n\nSaved revision: ${revision}\nContent hash: ${info.contentHash}\n\nThis archive contains data and original resources, not an executable app.\nThe same project identity and files are used for both platforms.\n\nIn a matching Desktop Buddy Studio source checkout with its pinned toolchain:\n\n1. Extract this archive into projects/.\n2. Run npm ci --ignore-scripts --no-audit --no-fund.\n3. Run node scripts/build-project.mjs projects/${meta.id} --development.\n\nRun on Apple Silicon macOS for .app/DMG; Windows x64 for the NSIS .exe.\nTo build both, commit projects/${meta.id} and run the Build project for macOS and Windows workflow with project_path=projects/${meta.id}.\nCI artifacts are development packages, without public signing/notarization or interactive OS verification.\nNo credentials, local drafts, build history or other projects are included.\n`);
      const archive = join(stage, "project.tar.gz");
      await run(tarExecutable, ["-czf", archive, "-C", stage, meta.id, "export.json", "BUILD.md"]);
      const dir = join(store.path(id), "exports");
      await fs.mkdir(dir, { recursive: true });
      const path = join(dir, `${revision}.tar.gz`);
      await fs.rename(archive, path);
      return { path, name: `${meta.id}-${revision.slice(0, 8)}.tar.gz` };
    } finally {
      await fs.rm(stage, { recursive: true, force: true });
    }
  });
}
