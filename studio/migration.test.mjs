import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import { randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import { join, resolve, basename } from "node:path";
import { ProjectStore, atomicJson } from "./store.mjs";
import { createStudio } from "./server.mjs";
const cli = resolve("target/debug/buddy");
async function fixture(t) {
  const dataRoot = await fs.mkdtemp(join(tmpdir(), ".buddy-package-test-"));
  t.after(() => fs.rm(dataRoot, { recursive: true, force: true }));
  const store = new ProjectStore({ dataRoot, cli });
  await store.init();
  return store;
}
test("package ID names directories, duplicate creation is rejected atomically and clone gets a readable new package", async (t) => {
  const store = await fixture(t);
  const results = await Promise.allSettled([
    store.create({ name: "My Cat", packageId: "com.example.mycat" }),
    store.create({ name: "Other Cat", packageId: "com.example.mycat" }),
  ]);
  assert.equal(results.filter((r) => r.status === "fulfilled").length, 1);
  assert.equal(results.find((r) => r.status === "rejected").reason.status, 409);
  const doc = await store.load("com.example.mycat");
  assert.equal(doc.meta.id, "com.example.mycat");
  assert.equal(doc.project.product.identifier, doc.meta.id);
  assert.equal(basename(store.path(doc.meta.id)), doc.meta.id);
  const copy = await store.clone(doc.meta.id);
  assert.equal(copy.meta.id, "com.example.mycat.copy");
  assert.notEqual(copy.project.projectId, doc.project.projectId);
  await assert.rejects(store.create({ packageId: "../escape" }));
  await assert.rejects(store.create({ packageId: "con.example.bad" }));
});
test("legacy UUID folders and URLs migrate without changing app identity, resources, revisions or build files; interrupted migration resumes", async (t) => {
  const store = await fixture(t),
    doc = await store.create({
      name: "Legacy Cat",
      packageId: "com.example.legacycat",
    });
  const legacy = randomUUID(),
    oldRoot = join(store.root, "projects", legacy),
    newRoot = store.path(doc.meta.id),
    jobId = randomUUID();
  await fs.rename(newRoot, oldRoot);
  const meta = { ...doc.meta, id: legacy };
  delete meta.storageVersion;
  delete meta.packageId;
  delete meta.legacyIds;
  await atomicJson(join(oldRoot, "project.json"), meta);
  await fs.mkdir(join(oldRoot, "builds", "release"), { recursive: true });
  await fs.writeFile(
    join(oldRoot, "builds", "release", "app.zip"),
    "keep bytes",
  );
  await atomicJson(join(oldRoot, "jobs", `${jobId}.json`), {
    id: jobId,
    projectId: legacy,
    status: "succeeded",
    result: { directory: join(oldRoot, "builds", "release") },
    downloads: [{ name: "app.zip", path: "release/app.zip" }],
  });
  const migrated = new ProjectStore({ dataRoot: store.root, cli });
  await migrated.init();
  const restored = await migrated.load(legacy);
  assert.equal(restored.meta.id, "com.example.legacycat");
  assert.equal(restored.meta.revision, doc.meta.revision);
  assert.equal(restored.contentHash, doc.contentHash);
  assert.equal(restored.project.projectId, doc.project.projectId);
  assert.equal(
    await fs.readFile(join(newRoot, "builds", "release", "app.zip"), "utf8"),
    "keep bytes",
  );
  const job = JSON.parse(
    await fs.readFile(join(newRoot, "jobs", `${jobId}.json`)),
  );
  assert.equal(job.projectId, doc.meta.id);
  assert.equal(job.result.directory, join(newRoot, "builds", "release"));
  assert.equal(
    job.result.originalDirectory,
    join(oldRoot, "builds", "release"),
  );
  assert.equal(
    JSON.parse(
      await fs.readFile(join(newRoot, "project.before-package-id.json")),
    ).id,
    legacy,
  );
  // Simulate interruption after the directory rename and before metadata commit.
  await atomicJson(join(newRoot, "project.json"), meta);
  await atomicJson(join(newRoot, "package-id-migration.json"), {
    version: 1,
    fromId: legacy,
    toId: doc.meta.id,
    status: "pending",
  });
  const resumed = new ProjectStore({ dataRoot: store.root, cli });
  await resumed.init();
  await resumed.migratePackageDirectories();
  assert.equal((await resumed.load(legacy)).contentHash, doc.contentHash);
  assert.equal(
    JSON.parse(await fs.readFile(join(newRoot, "package-id-migration.json")))
      .status,
    "complete",
  );
  const server = await createStudio({
    dataRoot: store.root,
    cli,
    port: 0,
    templatePack: "/unused",
  });
  t.after(() => server.close());
  const headers = { Authorization: `Bearer ${server.token}` };
  const response = await fetch(`${server.origin}/api/projects/${legacy}`, {
    headers,
  });
  assert.equal(response.status, 200);
  assert.equal((await response.json()).meta.id, doc.meta.id);
  const download = await fetch(
    `${server.origin}/api/projects/${legacy}/download/${jobId}/app.zip`,
    { headers },
  );
  assert.equal(download.status, 200);
  assert.equal(await download.text(), "keep bytes");
});
test("a package collision preserves every source folder and file", async (t) => {
  const store = await fixture(t),
    doc = await store.create({
      name: "Collision",
      packageId: "com.example.collision",
    }),
    legacy = randomUUID();
  const oldRoot = join(store.root, "projects", legacy);
  await fs.cp(store.path(doc.meta.id), oldRoot, { recursive: true });
  await atomicJson(join(oldRoot, "project.json"), { ...doc.meta, id: legacy });
  const next = new ProjectStore({ dataRoot: store.root, cli });
  await assert.rejects(next.init(), /exists|Duplicate/);
  assert.equal((await store.load(doc.meta.id)).contentHash, doc.contentHash);
  assert.equal(
    JSON.parse(await fs.readFile(join(oldRoot, "project.json"))).id,
    legacy,
  );
});
