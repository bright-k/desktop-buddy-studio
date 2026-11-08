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
function wavFixture() {
  const wav = Buffer.alloc(44 + 800);
  wav.write("RIFF", 0);
  wav.writeUInt32LE(wav.length - 8, 4);
  wav.write("WAVEfmt ", 8);
  wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20);
  wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(8000, 24);
  wav.writeUInt32LE(8000, 28);
  wav.writeUInt16LE(1, 32);
  wav.writeUInt16LE(8, 34);
  wav.write("data", 36);
  wav.writeUInt32LE(wav.length - 44, 40);
  wav.fill(128, 44);
  return wav;
}
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
  const store = await fixture(t);
  const appRoot = join(store.root, "pinned-frontend");
  await fs.mkdir(appRoot);
  await fs.writeFile(join(appRoot, "index.html"), "<title>Pinned frontend</title>");
  const studio = await createStudio({
      dataRoot: store.root,
      cli,
      port: 0,
      appRoot,
      version: "fixture-engine-and-frontend",
      templatePack: join(store.root, "missing-template"),
      development: true,
    });
  t.after(() => studio.close());
  await assert.rejects(
    createStudio({ dataRoot: store.root, cli, port: 0 }),
    (error) => error.status === 409 && error.existingUrl === studio.url
      && error.existingVersion === "fixture-engine-and-frontend",
  );
  assert.match(await (await fetch(studio.origin)).text(), /Pinned frontend/);
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
  const uploaded = await request(prefix + "/resources?filename=blink.gif", {
    method: "POST",
    body: await fs.readFile("examples/media-resources/blink.gif"),
  });
  assert.equal(uploaded.status, 201);
  const gif = await uploaded.json();
  const first = await request(prefix + `/gif-frame/${gif.name}/0`);
  const second = await request(prefix + `/gif-frame/${gif.name}/1`);
  assert.equal(first.status, 200);
  assert.equal(second.status, 200);
  assert.equal(second.headers.get("content-type"), "image/png");
  const firstPng = Buffer.from(await first.arrayBuffer());
  const secondPng = Buffer.from(await second.arrayBuffer());
  assert.notDeepEqual(firstPng, secondPng);
  assert.equal(secondPng.subarray(1, 4).toString(), "PNG");
  const wav = wavFixture();
  const audioUpload = await request(prefix + "/resources?filename=ping.wav", {
    method: "POST",
    body: wav,
  });
  assert.equal(audioUpload.status, 201);
  const sound = await audioUpload.json();
  assert.equal(sound.kind, "audio");
  d.character.sounds = { ping: { audio: `resources/${sound.name}` } };
  d.character.resources = { blink: { image: `resources/${gif.name}` } };
  d.character.animations["idle-blink"] = { resource: "blink", loop: true };
  d.character.defaultAnimation = "typing";
  d.project.idle = { cursorEnabled: true, directions: { upLeft: { animation: "idle-blink", mirror: false } } };
  Object.assign(d.project.bindings.find((b) => b.id === "jump"), {
    followCursor: true,
    sound: { resource: "ping", volume: 0.7 },
  });
  const soundSave = await request(prefix, { method: "PUT", body: JSON.stringify(payload(d)) });
  assert.equal(soundSave.status, 200);
  const saved = await soundSave.json();
  const response = await request(prefix + `/resources/${sound.name}`);
  assert.equal(response.headers.get("content-type"), "audio/wav");
  assert.deepEqual(Buffer.from(await response.arrayBuffer()), wav);
  const inspect = await store.inspectPath(await store.currentPath(d.meta.id));
  assert.equal(inspect.project.bindings.find((b) => b.id === "jump").sound.resource, "ping");
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
        events: [
          { atMs: 50, event: { type: "cursorMove", x: 120, y: 120 } },
          { atMs: 100, event: { type: "keyDown", code: "Space" } },
          { atMs: 600, event: { type: "cursorMove", x: 228, y: 228 } },
        ],
        untilMs: 650,
      }),
    })
  ).json();
  assert.equal(timeline[0].state.animation, "typing");
  assert.equal(timeline.find((f) => f.atMs === 50).state.animation, "idle-blink");
  assert.equal(timeline.find((f) => f.atMs === 50).state.facingLeft, false);
  const jump = timeline.find((f) => f.state.animation === "jump").state;
  assert.equal(jump.facingLeft, true);
  assert.deepEqual(jump.sound, { resource: "ping", volume: 0.7 });
  assert.equal(timeline.at(-1).state.animation, "typing");
  assert.equal(timeline.at(-1).state.sound, undefined);
  const queued = await (
    await request(prefix + "/builds", {
      method: "POST",
      body: JSON.stringify({ revision: saved.meta.revision, development: true }),
    })
  ).json();
  assert.equal(queued.revision, saved.meta.revision);
  // The missing template must fail at template verification on every OS.
  const deadline = Date.now() + 10000;
  let finished;
  while (Date.now() < deadline) {
    const [job] = await studio.jobs.list(d.meta.id);
    if (job.finishedAt) {
      finished = job;
      break;
    }
    await new Promise((r) => setTimeout(r, 30));
  }
  assert.equal(finished?.status, "failed");
  assert.match(finished.error, /^verify-template\r?$/m);
  assert.doesNotMatch(finished.error, /^configure-product\r?$/m);
});

test("legacy cursor bindings in saved projects and drafts appear as idle settings without changing the stored revision", async (t) => {
  const store = await fixture(t);
  const created = await store.create();
  const legacy = { id: "legacy-look", trigger: { type: "cursorMove" }, effect: {
    type: "faceCursor", directions: { up: { animation: "walk", mirror: false } },
  } };
  created.project.bindings.unshift(legacy);
  const saved = await store.save(created.meta.id, payload(created));
  const raw = await store.inspectPath(await store.currentPath(saved.meta.id));
  assert.equal(raw.project.bindings[0].id, legacy.id);
  assert.equal(saved.project.bindings.some((b) => b.id === legacy.id), false);
  assert.deepEqual(saved.project.idle, { cursorEnabled: true, directions: legacy.effect.directions });
  assert.equal(saved.sequences.length, saved.project.bindings.length);

  const draft = structuredClone(raw.project);
  draft.bindings[0].effect.directions.up.animation = "typing";
  await store.saveDraft(saved.meta.id, { ...payload(saved), project: draft, draftRevision: null });
  const reloaded = await store.load(saved.meta.id);
  assert.equal(reloaded.project.idle.directions.up.animation, "walk");
  assert.equal(reloaded.draft.project.idle.directions.up.animation, "typing");
  assert.equal(reloaded.draft.project.bindings.length, saved.project.bindings.length);
});
