import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createStudio } from "./server.mjs";
import { ProjectStore, scopedFile } from "./store.mjs";
import { run } from "./cli.mjs";
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

test("an empty project saves before its first sprite and builds a render plan from the imported GIF alone", async (t) => {
  const store = await fixture(t);
  const doc = await store.create({ name: "Empty Buddy", template: "empty" });
  const blankPath = await store.currentPath(doc.meta.id);
  assert.equal(doc.character.image, undefined);
  assert.equal(doc.character.defaultAnimation, "");
  assert.deepEqual(doc.character.animations, {});
  assert.deepEqual(doc.images, []);
  await assert.rejects(fs.access(join(blankPath, "character", "sprite.png")), { code: "ENOENT" });
  await assert.rejects(run(cli, ["validate", blankPath]), /defaultAnimation/);

  doc.project.timer = { enabled: true };
  const blank = await store.save(doc.meta.id, payload(doc));
  assert.equal((await store.load(doc.meta.id)).project.timer.enabled, true);
  assert.equal((await store.clone(doc.meta.id)).character.image, undefined);

  const gif = await store.upload(doc.meta.id, await fs.readFile("examples/media-resources/blink.gif"), "blink.gif");
  blank.character.resources = { blink: { image: `resources/${gif.name}` } };
  blank.character.animations.idle = { resource: "blink", frames: [], loop: true, useSourceTiming: true };
  blank.character.defaultAnimation = "idle";
  const ready = await store.save(doc.meta.id, payload(blank));
  assert.equal(ready.character.image, undefined);
  assert.equal(ready.renderPlan.animations.idle.frames.length, gif.frames);
  ready.character.resources.blink.name = "기본 눈 깜박임";
  const renamed = await store.save(doc.meta.id, payload(ready));
  assert.deepEqual(renamed.character.resources.blink, {
    image: `resources/${gif.name}`, name: "기본 눈 깜박임",
  });
  assert.equal(renamed.character.animations.idle.resource, "blink");
  const readyPath = await store.currentPath(doc.meta.id);
  const snapshot = join(store.root, "sprite-only");
  await run(cli, ["snapshot", readyPath, "--output", snapshot]);
  await assert.rejects(fs.access(join(snapshot, "character", "sprite.png")), { code: "ENOENT" });
  const copy = await store.clone(doc.meta.id);
  assert.equal(copy.renderPlan.animations.idle.frames.length, gif.frames);
  assert.equal(copy.character.resources.blink.name, "기본 눈 깜박임");
});

test("a legacy sprite can be removed only after its last animation reference is replaced", async (t) => {
  const store = await fixture(t);
  const doc = await store.create({ name: "Legacy Buddy", template: "typing-cat" });
  const legacyPath = await store.currentPath(doc.meta.id);
  delete doc.character.image;
  await assert.rejects(store.save(doc.meta.id, payload(doc)), /legacy frames/);
  assert.equal((await store.load(doc.meta.id)).character.image, "sprite.png");

  const gif = await store.upload(doc.meta.id, await fs.readFile("examples/media-resources/blink.gif"), "blink.gif");
  doc.character.resources = { blink: { image: `resources/${gif.name}` } };
  for (const clip of Object.values(doc.character.animations))
    Object.assign(clip, { resource: "blink", frames: [], regions: [] });
  const saved = await store.save(doc.meta.id, payload(doc));
  assert.equal(saved.character.image, undefined);
  await assert.rejects(fs.access(join(await store.currentPath(doc.meta.id), "character", "sprite.png")), { code: "ENOENT" });
  await fs.access(join(legacyPath, "character", "sprite.png"));
  const restored = await store.restore(doc.meta.id, doc.meta.revision, saved.meta.revision, null);
  assert.equal(restored.character.image, "sprite.png");
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
  d.character.sounds = { ping: { audio: `resources/${sound.name}`, name: "완료 소리" } };
  d.character.resources = { blink: { image: `resources/${gif.name}` } };
  d.character.animations["idle-blink"] = { resource: "blink", loop: true };
  d.character.defaultAnimation = "typing";
  d.project.idle = { cursorEnabled: true, directions: { upLeft: { animation: "idle-blink", mirror: false } } };
  const guideBody = "<!doctype html><title>Buddy guide</title><p>설치 안내</p>";
  const guideUpload = await request(prefix + "/resources?purpose=installer&filename=guide.html", {
    method: "POST", body: guideBody,
  });
  assert.equal(guideUpload.status, 201);
  const guide = await guideUpload.json();
  d.project.build.macosDmg = { guide: `assets/installer/${guide.name}` };
  d.project.cursorFollow = { enabled: false, stopDistanceDip: 0, bands: [{ minDistanceDip: 0, speedDipPerSecond: 120, animation: "walk" }] };
  Object.assign(d.project.bindings.find((b) => b.id === "jump"), {
    followCursor: true,
    sound: { resource: "ping", volume: 0.7 },
  });
  const soundSave = await request(prefix, { method: "PUT", body: JSON.stringify(payload(d)) });
  assert.equal(soundSave.status, 200);
  const saved = await soundSave.json();
  assert.equal(saved.character.sounds.ping.name, "완료 소리");
  assert.equal(saved.project.build.macosDmg.guide, d.project.build.macosDmg.guide);
  assert.equal(saved.project.cursorFollow.stopDistanceDip, 0);
  const guideDownload = await request(prefix.replace("/api/", "/%61pi/") + `/source/assets/installer/${guide.name}`);
  assert.equal(guideDownload.status, 200);
  assert.match(guideDownload.headers.get("content-disposition"), /^attachment;/);
  assert.equal(guideDownload.headers.get("content-type"), "application/octet-stream");
  assert.equal(await guideDownload.text(), guideBody);
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
  saved.project.timer = { enabled: true };
  saved.project.idleGroups = [
    { id: "work", name: "작업", states: ["countdown", "focus"], animation: "walk", cursorMode: "custom",
      directions: { upLeft: { animation: "idle-blink", mirror: true } } },
    { id: "rest", name: "휴식", states: ["shortBreak", "longBreak"], cursorMode: "inherit" },
  ];
  saved.project.bindings.push({
    id: "timer-start", trigger: { type: "timerStarted", phase: "countdown", alternatives: ["focus"] },
    effect: { type: "playAnimation", animation: "jump", durationMs: 100 },
  }, {
    id: "buddy-hold", trigger: { type: "buddyLongPress", holdMs: 600 },
    effect: { type: "playAnimation", animation: "click", durationMs: 100 },
  }, {
    id: "timer-end", trigger: { type: "timerCompleted", phase: "longBreak" },
    effect: { type: "playAnimation", animation: "celebrate", durationMs: 100 },
  }, {
    id: "typing-done",
    trigger: { type: "typingStopped", minTypingSeconds: 2, minKeys: 5, pauseMs: 700, minIntervalSeconds: 10 },
    effect: {
      type: "playAnimation",
      animation: "jump",
      variation: { mode: "cycle", alternatives: ["click"] },
    },
  }, {
    id: "idle-fidget",
    trigger: { type: "idleFor", seconds: 300 },
    effect: {
      type: "playAnimation",
      animation: "celebrate",
      variation: { mode: "cycle", alternatives: ["click"] },
    },
  });
  const timedSave = await request(prefix, { method: "PUT", body: JSON.stringify(payload(saved)) });
  assert.equal(timedSave.status, 200);
  const timed = await timedSave.json();
  assert.equal(timed.project.bindings.at(-1).trigger.seconds, 300);
  assert.deepEqual(timed.project.idleGroups, saved.project.idleGroups);
  const overlapping = structuredClone(timed);
  overlapping.project.idleGroups[1].states.push("focus");
  const rejected = await request(prefix, { method: "PUT", body: JSON.stringify(payload(overlapping)) });
  assert.equal(rejected.ok, false);
  const error = (await rejected.json()).error;
  assert.match(error, /idleGroups\[1\]\.states/);
  assert.match(error, /작업.*휴식/);
  assert.equal((await store.metadata(d.meta.id)).revision, timed.meta.revision);
  const timerPreview = await request(prefix + "/simulate", {
    method: "POST",
    body: JSON.stringify({ untilMs: 550, includeBindingIds: true, events: [
      { atMs: 0, event: { type: "timerPhaseChanged", phase: "focus" } },
      { atMs: 10, event: { type: "timerStarted", phase: "focus" } },
      { atMs: 50, event: { type: "cursorMove", x: 120, y: 120 } },
      { atMs: 200, event: { type: "buddyLongPress" } },
      { atMs: 400, event: { type: "timerPhaseChanged", phase: null } },
      { atMs: 400, event: { type: "timerCompleted", phase: "longBreak" } },
    ] }),
  });
  assert.equal(timerPreview.status, 200);
  const timerFrames = await timerPreview.json();
  assert.equal(timerFrames.find(frame => frame.atMs === 10).bindingId, "timer-start");
  const groupPose = timerFrames.find(frame => frame.atMs === 150).state;
  assert.equal(groupPose.animation, "idle-blink");
  assert.equal(groupPose.facingLeft, true);
  assert.equal(timerFrames.find(frame => frame.atMs === 200).bindingId, "buddy-hold");
  assert.equal(timerFrames.find(frame => frame.atMs === 400).bindingId, "timer-end");
  assert.equal(timerFrames.at(-1).state.facingLeft, false);
  const idlePreview = await request(prefix + "/simulate", {
    method: "POST",
    body: JSON.stringify({ fromMs: 299900, untilMs: 300100, events: [] }),
  });
  assert.equal(idlePreview.status, 200);
  const idleFrames = await idlePreview.json();
  assert.equal(idleFrames[0].atMs, 0);
  assert.equal(idleFrames[0].state.animation, "typing");
  assert.equal(idleFrames.find((frame) => frame.atMs === 100).state.animation, "celebrate");
  const motionPreview = { bindingId: "idle-fidget", revision: timed.meta.revision, seed: 42, starts: 1 };
  const nextMotion = await request(prefix + "/simulate", {
    method: "POST",
    body: JSON.stringify({ fromMs: 299900, untilMs: 300100, events: [], motionPreview }),
  });
  assert.equal(nextMotion.status, 200);
  const selected = (await nextMotion.json()).find((frame) => frame.atMs === 100);
  assert.equal(selected.bindingId, "idle-fidget");
  assert.equal(selected.state.animation, "click");
  const staleMotion = await request(prefix + "/simulate", {
    method: "POST",
    body: JSON.stringify({
      fromMs: 299900, untilMs: 300100, events: [],
      motionPreview: { ...motionPreview, revision: saved.meta.revision },
    }),
  });
  assert.equal(staleMotion.status, 409);
  const typingInputs = [0, 500, 1000, 1500, 2000, 2700].flatMap(atMs => [
    { atMs, event: { type: "keyDown", code: "KeyJ" } },
    { atMs: atMs + 5, event: { type: "keyUp", code: "KeyJ" } },
  ]);
  const typingPreview = await request(prefix + "/simulate", {
    method: "POST",
    body: JSON.stringify({
      fromMs: 2600, untilMs: 3500, events: typingInputs,
      motionPreview: { ...motionPreview, bindingId: "typing-done" },
    }),
  });
  assert.equal(typingPreview.status, 200);
  const finishedTyping = (await typingPreview.json()).find(frame => frame.bindingId === "typing-done");
  assert.equal(finishedTyping.atMs, 800);
  assert.equal(finishedTyping.state.animation, "click");
  const queued = await (
    await request(prefix + "/builds", {
      method: "POST",
      body: JSON.stringify({ revision: timed.meta.revision, development: true }),
    })
  ).json();
  assert.equal(queued.revision, timed.meta.revision);
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
  delete timed.project.build.macosDmg;
  const removed = await request(prefix, { method: "PUT", body: JSON.stringify(payload(timed)) });
  assert.equal(removed.status, 200);
  assert.equal((await request(prefix + `/source/assets/installer/${guide.name}`)).status, 404);
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

function pdfGuideFixture() {
  let pdf = "%PDF-1.4\n";
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] >>",
  ];
  const offsets = objects.map((object, index) => {
    const offset = pdf.length;
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
    return offset;
  });
  return pdf + `xref\n0 4\n0000000000 65535 f \n${offsets.map(offset =>
    `${String(offset).padStart(10, "0")} 00000 n \n`).join("")}trailer\n<< /Root 1 0 R /Size 4 >>\nstartxref\n${pdf.length}\n%%EOF\n`;
}

test("PDF guides roundtrip and build deletion distinguishes records, files, and active builds", async (t) => {
  const store = await fixture(t);
  const d = await store.create({ name: "PDF Buddy" });
  const studio = await createStudio({ dataRoot: store.root, cli, port: 0 });
  t.after(() => studio.close());
  const prefix = `${studio.origin}/api/projects/${d.meta.id}`;
  const request = (path, options = {}) => fetch(prefix + path, {
    ...options, headers: { Authorization: `Bearer ${studio.token}` },
  });
  const upload = (body, installer) => request(`/resources?filename=guide.pdf${installer ? "&purpose=installer" : ""}`, { method: "POST", body });
  const pdf = pdfGuideFixture();
  assert.equal((await upload(pdf, false)).status, 400);
  assert.equal((await upload("%PDF-1.4\ntruncated", true)).status, 400);
  const accepted = await upload(pdf, true);
  assert.equal(accepted.status, 201);
  const guide = await accepted.json();
  const guidePath = `assets/installer/${guide.name}`;
  d.project.build.macosDmg = { background: guidePath };
  await assert.rejects(store.save(d.meta.id, payload(d)), /INVALID_INSTALL_ASSET/);
  d.project.build.macosDmg = { guide: guidePath, guidePosition: { x: 180, y: 180 } };
  await assert.rejects(store.save(d.meta.id, payload(d)), /guidePosition/);
  d.project.build.macosDmg.guidePosition = { x: 340, y: 260 };
  const saved = await store.save(d.meta.id, payload(d));
  assert.deepEqual(saved.project.build.macosDmg.guidePosition, { x: 340, y: 260 });
  const downloaded = await request(`/source/${guidePath}`);
  assert.equal(downloaded.status, 200);
  assert.equal(downloaded.headers.get("content-type"), "application/pdf");
  assert.match(downloaded.headers.get("content-disposition"), /^attachment;/);
  assert.equal(await downloaded.text(), pdf);

  const [localId, remoteId, activeId, builderId] = [1, 2, 3, 4]
    .map(n => `00000000-0000-4000-8000-00000000000${n}`);
  const root = join(store.path(d.meta.id), "builds");
  const localDir = join(root, `pdf-buddy-0.1.0-${builderId}`), remoteDir = join(root, `actions-${remoteId}`);
  const build = { projectId: d.meta.id, revision: saved.meta.revision, createdAt: new Date().toISOString(), status: "succeeded" };
  for (const directory of [localDir, remoteDir]) {
    await fs.mkdir(directory, { recursive: true });
    await fs.writeFile(join(directory, "buddy.dmg"), "test build");
  }
  await fs.writeFile(join(root, `build-${builderId}.log`), "build log");
  await studio.jobs.persist({ ...build, id: localId, result: { directory: localDir, jobId: builderId } });
  await studio.jobs.persist({ ...build, id: remoteId, provider: "github", result: { directory: remoteDir } });
  await studio.jobs.persist({ ...build, id: activeId, provider: "github", status: "awaiting_auth" });
  const remove = (id, files) => request(`/builds/${id}${files ? "?files=true" : ""}`, { method: "DELETE" });
  assert.equal((await remove(activeId, true)).status, 409);
  assert.equal((await remove(remoteId, false)).status, 200);
  assert.equal(await fs.readFile(join(remoteDir, "buddy.dmg"), "utf8"), "test build");
  assert.equal((await remove(localId, true)).status, 200);
  assert.equal((await remove(localId, true)).status, 404);
  await assert.rejects(fs.stat(localDir), { code: "ENOENT" });
  await assert.rejects(fs.stat(join(root, `build-${builderId}.log`)), { code: "ENOENT" });
  assert.deepEqual((await (await request("/builds")).json()).map(job => job.id), [activeId]);
  assert.equal((await store.metadata(d.meta.id)).builtSlug, saved.project.product.slug);
  saved.project.product.slug += "-renamed";
  await assert.rejects(store.save(d.meta.id, payload(saved)), /실행 파일 이름/);
});
