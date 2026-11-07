import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { ProjectStore } from "./store.mjs";
import { BuildQueue } from "./jobs.mjs";
const cli = resolve("target/debug/buddy");
async function fixture(t) {
  const root = await fs.mkdtemp(join(tmpdir(), ".buddy-draft-test-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const store = new ProjectStore({ dataRoot: root, cli });
  await store.init();
  return store;
}
const data = (d) => ({
  revision: d.meta.revision,
  project: d.project,
  character: d.character,
  license: d.license,
});
test("invalid edits survive restart as a separate draft, stale tabs cannot overwrite them, saving produces a validated revision", async (t) => {
  const store = await fixture(t),
    doc = await store.create({ name: "Draft Buddy" }),
    original = doc.contentHash;
  doc.project.product.name = "Unpublished Buddy";
  doc.character.animations.idle.fps = 0;
  const draft = await store.saveDraft(doc.meta.id, data(doc));
  const reloaded = new ProjectStore({ dataRoot: store.root, cli });
  await reloaded.init();
  const view = await reloaded.load(doc.meta.id);
  assert.equal(view.contentHash, original);
  assert.equal(view.project.product.name, "Draft Buddy");
  assert.equal(view.draft.project.product.name, "Unpublished Buddy");
  assert.equal(view.draft.character.animations.idle.fps, 0);
  await assert.rejects(
    reloaded.saveDraft(doc.meta.id, data(doc)),
    (e) => e.status === 409,
  );
  await assert.rejects(
    reloaded.save(doc.meta.id, data(doc)),
    (e) => e.status === 409,
  );
  doc.character.animations.idle.fps = 2;
  const saved = await reloaded.save(doc.meta.id, {
    ...data(doc),
    draftRevision: draft.draftRevision,
  });
  assert.equal(saved.draft, null);
  assert.equal(saved.project.product.name, "Unpublished Buddy");
  assert.notEqual(saved.meta.revision, doc.meta.revision);
  const next = await reloaded.saveDraft(saved.meta.id, data(saved));
  await reloaded.discardDraft(saved.meta.id, next.draftRevision);
  assert.equal((await reloaded.load(saved.meta.id)).draft, null);
  assert.equal(
    (await fs.readdir(join(reloaded.path(saved.meta.id), "drafts"))).length,
    2,
  );
});
test("double build clicks reuse the same queued revision while a different mode remains a separate job", async (t) => {
  const store = await fixture(t),
    doc = await store.create({ name: "Queue Buddy" }),
    queue = new BuildQueue(store, {
      templatePack: "/unused",
      development: true,
    });
  queue.pump = async () => {};
  const [first, second] = await Promise.all([
    queue.enqueue(doc.meta.id, doc.meta.revision, true),
    queue.enqueue(doc.meta.id, doc.meta.revision, true),
  ]);
  assert.equal(first.id, second.id);
  assert.equal((await queue.list(doc.meta.id)).length, 1);
  const other = await queue.enqueue(doc.meta.id, doc.meta.revision, false);
  assert.notEqual(first.id, other.id);
  await queue.close();
  assert.ok(
    (await queue.list(doc.meta.id)).every((j) => j.status === "cancelled"),
  );
});

test("restoring an earlier revision protects a newer draft and archives the confirmed draft", async (t) => {
  const store = await fixture(t);
  const original = await store.create({ name: "Original Buddy" });
  const id = original.meta.id;
  original.project.product.name = "Saved Buddy";
  const saved = await store.save(id, data(original));
  saved.project.product.name = "Local Draft";
  const first = await store.saveDraft(id, data(saved));
  saved.project.product.name = "Newer Draft";
  const latest = await store.saveDraft(id, {
    ...data(saved),
    draftRevision: first.draftRevision,
  });
  await assert.rejects(
    store.restore(id, original.meta.revision, saved.meta.revision, first.draftRevision),
    (error) => error.status === 409,
  );
  assert.equal((await store.metadata(id)).revision, saved.meta.revision);
  const restored = await store.restore(id, original.meta.revision, saved.meta.revision, latest.draftRevision);
  assert.equal(restored.project.product.name, "Original Buddy");
  assert.equal(restored.draft, null);
  const back = await store.restore(id, saved.meta.revision, restored.meta.revision);
  assert.equal(back.project.product.name, "Saved Buddy");
  assert.equal(back.draft, null);
  const backup = JSON.parse(await fs.readFile(join(store.path(id), "drafts", `restored-${latest.draftRevision}.json`), "utf8"));
  assert.equal(backup.project.product.name, "Newer Draft");
});

test("a project load keeps its metadata and preview tied to one saved revision", async (t) => {
  const store = await fixture(t);
  const original = await store.create({ name: "Original Buddy" });
  const metadata = store.metadata.bind(store);
  const read = Promise.withResolvers();
  const resume = Promise.withResolvers();
  let delay = true;
  store.metadata = async (id) => {
    const meta = await metadata(id);
    if (delay) {
      delay = false;
      read.resolve();
      await resume.promise;
    }
    return meta;
  };
  const loading = store.load(original.meta.id);
  await read.promise;
  try {
    original.project.product.name = "Saved While Loading";
    await store.save(original.meta.id, data(original));
  } finally {
    resume.resolve();
  }
  const loaded = await loading;
  assert.equal(loaded.meta.revision, original.meta.revision);
  assert.equal(loaded.project.product.name, loaded.meta.name);
  assert.equal(loaded.contentHash, original.contentHash);
});

test("a preview write failure leaves the saved revision and draft available for retry", async (t) => {
  const store = await fixture(t);
  const original = await store.create({ name: "Before Retry" });
  const id = original.meta.id;
  original.project.product.name = "After Retry";
  const draft = await store.saveDraft(id, data(original));
  const edit = { ...data(original), draftRevision: draft.draftRevision };
  const previews = join(store.path(id), "previews");
  await fs.rename(previews, `${previews}.backup`);
  await fs.writeFile(previews, "blocks creation of preview files");
  await assert.rejects(store.save(id, edit));
  assert.equal((await store.metadata(id)).revision, original.meta.revision);
  assert.equal((await store.readDraft(id)).draftRevision, draft.draftRevision);
  assert.equal((await store.revisions(id)).length, 1);
  await fs.rename(previews, `${previews}.blocked`);
  await fs.rename(`${previews}.backup`, previews);
  const saved = await store.save(id, edit);
  assert.equal(saved.project.product.name, "After Retry");
  assert.equal(saved.draft, null);
});
