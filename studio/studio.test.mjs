import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createStudio } from "./server.mjs";
import { ProjectStore, scopedFile } from "./store.mjs";
const cli = resolve("target/debug/buddy");
async function fixture(t) {
  const root = await fs.mkdtemp(join(tmpdir(), ".buddy-studio-test-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const store = new ProjectStore({ dataRoot: root, cli });
  await store.init();
  return store;
}
const payload = (d) => ({
  revision: d.meta.revision,
  project: d.project,
  character: d.character,
  license: d.license,
});
test("two projects isolate resources, save immutable revisions, reject stale writes, restore and clone with fresh identity", async (t) => {
  const store = await fixture(t),
    a = await store.create({ name: "First Buddy" }),
    b = await store.create({ name: "Second Buddy", template: "click-mascot" });
  assert.notEqual(a.project.product.identifier, b.project.product.identifier);
  const originalPath = await store.currentPath(a.meta.id);
  const resource = await store.upload(
    a.meta.id,
    await fs.readFile("examples/click-mascot/character/sprite.png"),
    "other.png",
  );
  a.character.resources = { other: { image: `resources/${resource.name}` } };
  a.character.animations.idle = {
    resource: "other",
    frames: [],
    regions: [{ x: 256, y: 0, width: 256, height: 256 }],
    fps: 3,
    loop: true,
  };
  a.project.defaults.timing.idleTimeoutMs = 150;
  const saved = await store.save(a.meta.id, payload(a));
  assert.notEqual(saved.meta.revision, a.meta.revision);
  assert.equal(saved.renderPlan.animations.idle.frames[0].region.x, 256);
  assert.equal((await store.load(b.meta.id)).library[resource.name], undefined);
  assert.equal(
    JSON.parse(
      await fs.readFile(join(originalPath, "buddy.project.json"), "utf8"),
    ).defaults.timing.idleTimeoutMs,
    250,
  );
  await assert.rejects(
    store.save(a.meta.id, payload(a)),
    (e) => e.status === 409,
  );
  const copy = await store.clone(a.meta.id);
  assert.notEqual(copy.project.projectId, saved.project.projectId);
  assert.notEqual(
    copy.project.product.identifier,
    saved.project.product.identifier,
  );
  assert.equal(copy.renderPlan.animations.idle.frames[0].region.x, 256);
  const restored = await store.restore(
    a.meta.id,
    a.meta.revision,
    saved.meta.revision,
  );
  assert.equal(restored.project.defaults.timing.idleTimeoutMs, 250);
  assert.ok(restored.library[resource.name]);
  await store.archive(a.meta.id, true);
  assert.equal(
    (await store.list()).find((p) => p.id === a.meta.id).archived,
    true,
  );
  await assert.rejects(store.save(a.meta.id, payload(restored)), /보관/);
  await store.archive(a.meta.id, false);
  assert.equal((await store.metadata(a.meta.id)).archived, false);
});
test("invalid definition cannot replace the saved revision, symlink file escape is denied", async (t) => {
  const store = await fixture(t),
    d = await store.create();
  const original = d.meta.revision;
  d.character.animations.idle.frames = [9999];
  await assert.rejects(store.save(d.meta.id, payload(d)), /frameCount/);
  assert.equal((await store.metadata(d.meta.id)).revision, original);
  await assert.rejects(
    store.upload(d.meta.id, Buffer.from("bad"), "broken.gif"),
  );
  await assert.rejects(
    store.upload(d.meta.id, Buffer.from("bad"), "script.svg"),
    /PNG/,
  );
  const dir = join(store.path(d.meta.id), "resources");
  await fs.mkdir(dir, { recursive: true });
  await fs.symlink(
    join(store.path(d.meta.id), "project.json"),
    join(dir, "escape.png"),
  );
  await assert.rejects(scopedFile(dir, "escape.png"), (e) => e.status === 403);
  await assert.rejects(scopedFile(dir, "../project.json"));
});
test("HTTP requires session authorization and same origin; actual engine simulation and keyword compiler are used", async (t) => {
  const store = await fixture(t),
    studio = await createStudio({
      dataRoot: store.root,
      cli,
      port: 0,
      templatePack: "/unused",
      development: true,
    });
  t.after(() => studio.close());
  await assert.rejects(
    createStudio({ dataRoot: store.root, cli, port: 0 }),
    (error) => error.status === 409 && error.existingUrl === studio.url,
  );
  const request = (path, options = {}) =>
    fetch(studio.origin + path, {
      ...options,
      headers: { Authorization: `Bearer ${studio.token}`, ...options.headers },
    });
  assert.equal((await fetch(studio.origin + "/api/projects")).status, 401);
  assert.equal(
    (
      await request("/api/projects", {
        headers: { Origin: "https://example.com" },
      })
    ).status,
    403,
  );
  const session = await request("/api/session", { method: "POST" });
  assert.match(session.headers.get("set-cookie"), /HttpOnly; SameSite=Strict/);
  const d = await (
    await request("/api/projects", {
      method: "POST",
      body: JSON.stringify({ name: "Web Buddy" }),
    })
  ).json();
  const prefix = `/api/projects/${d.meta.id}`;
  const seq = await (
    await request(prefix + "/sequence", {
      method: "POST",
      body: JSON.stringify({ index: 4 }),
    })
  ).json();
  assert.equal(seq.length, 7);
  const timeline = await (
    await request(prefix + "/simulate", {
      method: "POST",
      body: JSON.stringify({
        events: [{ atMs: 0, event: { type: "keyDown", code: "Space" } }],
        untilMs: 500,
      }),
    })
  ).json();
  assert.ok(timeline.some((f) => f.state.animation === "jump"));
  assert.equal(timeline.at(-1).state.animation, "idle");
  const queued = await (
    await request(prefix + "/builds", {
      method: "POST",
      body: JSON.stringify({ revision: d.meta.revision, development: true }),
    })
  ).json();
  assert.equal(queued.revision, d.meta.revision);
  // Invalid factory path deliberately fails before launching an app build.
  const deadline = Date.now() + 10000;
  while (Date.now() < deadline) {
    const jobs = await studio.jobs.list(d.meta.id);
    if (jobs[0].status === "failed") {
      assert.match(jobs[0].error, /unused|manifest|No such file/);
      break;
    }
    await new Promise((r) => setTimeout(r, 30));
  }
  assert.equal((await studio.jobs.list(d.meta.id))[0].status, "failed");
});
