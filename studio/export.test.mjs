import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { ProjectStore } from "./store.mjs";
import { exportProject } from "./export.mjs";
import { run } from "./cli.mjs";
const cli = resolve("target/debug", process.platform === "win32" ? "buddy.exe" : "buddy");

test("portable export preserves saved hash, identity and GIF, excludes drafts and unrelated files, rejects stale revisions", async (t) => {
  const root = await fs.mkdtemp(join(tmpdir(), ".buddy-export-test-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const store = new ProjectStore({ dataRoot: join(root, "data"), cli });
  await store.init();
  const first = await store.create({ name: "Portable Buddy", packageId: "com.example.portable" });
  const gif = await fs.readFile("examples/media-resources/blink.gif");
  const resource = await store.upload(first.meta.id, gif, "blink.gif");
  first.character.resources = { blink: { image: `resources/${resource.name}` } };
  first.character.animations.celebrate = {
    resource: "blink", frames: [], regions: [], fps: 12, loop: false, useSourceTiming: true,
  };
  const data = (d) => ({ revision: d.meta.revision, project: d.project, character: d.character, license: d.license });
  const saved = await store.save(first.meta.id, data(first));
  saved.character.animations.idle.fps = 0;
  await store.saveDraft(saved.meta.id, data(saved));
  await fs.writeFile(join(await store.currentPath(saved.meta.id), "private-unreferenced.txt"), "not project data");
  const archive = await exportProject(store, saved.meta.id, saved.meta.revision);
  const list = await run("tar", ["-tzf", archive.path]);
  assert.match(list, /BUILD.md/);
  assert.match(list, /export.json/);
  assert.doesNotMatch(list, /private-unreferenced|draft|jobs|builds|resources\/index/);
  const extracted = join(root, "extracted");
  await fs.mkdir(extracted);
  await run("tar", ["-xzf", archive.path, "-C", extracted]);
  const info = JSON.parse(await run(cli, ["validate", join(extracted, saved.meta.id), "--json"]));
  assert.equal(info.contentHash, saved.contentHash);
  assert.equal(info.identifier, saved.meta.id);
  const project = JSON.parse(await fs.readFile(join(extracted, saved.meta.id, "buddy.project.json"), "utf8"));
  assert.equal(project.projectId, saved.project.projectId);
  assert.deepEqual(project.build.targets.sort(), ["macos-arm64", "windows-x64"]);
  assert.deepEqual(await fs.readFile(join(extracted, saved.meta.id, "character/resources", resource.name)), gif);
  const manifest = JSON.parse(await fs.readFile(join(extracted, "export.json"), "utf8"));
  assert.equal(manifest.contentHash, saved.contentHash);
  assert.equal((await store.load(saved.meta.id)).draft.character.animations.idle.fps, 0);
  await assert.rejects(exportProject(store, saved.meta.id, first.meta.revision), (e) => e.status === 409);
});
