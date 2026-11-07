import { after, test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import ts from 'typescript';

const temp = await mkdtemp(join(tmpdir(), 'buddy-renderer-tests-'));
after(() => rm(temp, { recursive: true, force: true }));
const source = await readFile(new URL('../packages/renderer/src/index.ts', import.meta.url), 'utf8');
await writeFile(join(temp, 'renderer.mjs'), ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText);
const { CharacterRenderer } = await import(pathToFileURL(join(temp, 'renderer.mjs')));
const original = JSON.parse(await readFile(new URL('../examples/typing-cat/character/manifest.json', import.meta.url)));

async function playerModule(name) {
  const source = await readFile(new URL(`../apps/player/src/${name}.ts`, import.meta.url), 'utf8');
  const file = join(temp, `${name}.mjs`);
  await writeFile(file, ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText);
  return import(pathToFileURL(file));
}
const { mergeSettings, settingsUpdate } = await playerModule('settings-model');
const { installCharacterDrag } = await playerModule('drag');

function harness(t, options = {}) {
  let now = 0, next = 1, draws = 0, errors = 0;
  const frames = new Map(), timers = new Map(), images = [], acknowledgements = [];
  const globals = {
    performance: { now: () => now },
    window: { devicePixelRatio: 1, setTimeout: (callback, delay) => {
      const id = next++; timers.set(id, { callback, due: now + delay }); return id;
    } },
    clearTimeout: id => timers.delete(id),
    requestAnimationFrame: callback => { const id = next++; frames.set(id, callback); return id; },
    cancelAnimationFrame: id => frames.delete(id),
    Image: class { constructor() { images.push(this); } },
  };
  const descriptors = Object.fromEntries(Object.keys(globals).map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]));
  for (const [key, value] of Object.entries(globals)) Object.defineProperty(globalThis, key, { value, configurable: true, writable: true });
  t.after(() => {
    for (const [key, descriptor] of Object.entries(descriptors)) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    }
  });
  const drawCalls = [];
  const context = { setTransform() {}, clearRect() {}, fillRect() {}, fillText() { errors++; },
    drawImage(...args) { drawCalls.push(args); if (context.fail) throw new Error('draw failed'); draws++; } };
  const canvas = { width: 0, height: 0, style: {}, getContext: () => context };
  const character = structuredClone(original);
  character.animations.idle = { frames: [0], fps: 30, loop: true };
  const renderer = new CharacterRenderer(canvas, character, options.images ?? 'test.png', ack => acknowledgements.push(ack), options.plan);
  renderer.resize(256, 256);
  const state = (actionInstanceId, epoch = 1) => ({ epoch, actionInstanceId, animation: 'idle', animationElapsedMs: 0, facingLeft: false });
  return { renderer, state, images, context, acknowledgements, canvas, drawCalls,
    get draws() { return draws; }, get errors() { return errors; },
    get scheduled() { return frames.size + timers.size; },
    step(time) {
      now = time;
      for (const [id, timer] of [...timers]) if (timer.due <= now) { timers.delete(id); timer.callback(); }
      const ready = [...frames.values()]; frames.clear();
      for (const callback of ready) callback(now);
    },
  };
}

test('restarts with identical pixels are acknowledged after an actual draw, capped at 30 FPS', t => {
  const h = harness(t);
  h.renderer.update(h.state(1), 1); h.images[0].onload(); h.step(0);
  h.renderer.update(h.state(2), 2); h.step(10);
  assert.equal(h.acknowledgements.length, 1);
  h.step(34);
  assert.equal(h.draws, 2);
  assert.deepEqual(h.acknowledgements.map(ack => [ack.seq, ack.actionInstanceId, ack.outcome]), [[1, 1, 'drawn'], [2, 2, 'drawn']]);
  h.renderer.update(h.state(2), 3); h.step(70);
  assert.equal(h.draws, 2);
  assert.equal(h.scheduled, 0);
  h.renderer.update(h.state(2, 2), 4); h.step(104);
  assert.equal(h.draws, 3);
  assert.equal(h.acknowledgements[2].epoch, 2);
});

test('superseded states waiting for the image never receive a fictitious ACK', t => {
  const h = harness(t);
  h.renderer.update(h.state(1), 10); h.step(0);
  h.renderer.update(h.state(2), 11); h.images[0].onload(); h.step(20);
  assert.equal(h.draws, 1);
  assert.equal(h.acknowledgements.length, 1);
  assert.equal(h.acknowledgements[0].seq, 11);
  assert.equal(h.acknowledgements[0].actionInstanceId, 2);
});

test('hidden states do not draw or acknowledge, and resuming uses the latest state', t => {
  const h = harness(t);
  h.images[0].onload(); h.renderer.setVisible(false);
  h.renderer.update(h.state(1), 1); h.step(50);
  h.renderer.update(h.state(2), 2); h.step(100);
  assert.equal(h.draws, 0); assert.equal(h.acknowledgements.length, 0); assert.equal(h.scheduled, 0);
  h.renderer.setVisible(true); h.step(150);
  assert.equal(h.acknowledgements[0].actionInstanceId, 2);
});

test('image errors acknowledge an error once and recovery separately acknowledges drawing', t => {
  const h = harness(t);
  h.images[0].onerror(); h.step(0);
  assert.equal(h.acknowledgements.length, 0);
  h.renderer.update(h.state(1), 1); h.step(10);
  h.renderer.resize(512, 512); h.step(20);
  assert.deepEqual(h.acknowledgements.map(ack => ack.outcome), ['error']);
  assert.equal(h.draws, 0);
  h.images[0].onload(); h.step(30);
  assert.deepEqual(h.acknowledgements.map(ack => ack.outcome), ['error', 'drawn']);
  assert.equal(h.draws, 1);
  h.context.fail = true; h.renderer.update(h.state(2), 2); h.step(70);
  assert.equal(h.acknowledgements.at(-1).outcome, 'error');
  assert.equal(h.acknowledgements.at(-1).actionInstanceId, 2);
});


test('prepared GIF timing, resource switching, crop coordinates and restart use the state clock', t => {
  const frame = (image, x, durationMs) => ({ image, region: { x, y: 3, width: 20, height: 30 }, durationMs });
  const h = harness(t, { images: { first: 'first.png', second: 'second.png' }, plan: { animations: { idle: { looping: true, frames: [frame('first', 10, 80), frame('second', 25, 240)] } } } });
  h.images.forEach(image => image.onload()); h.renderer.update(h.state(1), 1); h.step(0);
  assert.equal(h.drawCalls[0][0],h.images[0]); assert.deepEqual(h.drawCalls[0].slice(1,5),[10,3,20,30]);
  h.step(79); assert.equal(h.draws,1);
  h.step(80); assert.equal(h.drawCalls.at(-1)[0],h.images[1]);
  assert.deepEqual(h.drawCalls.at(-1).slice(1,5),[25,3,20,30]);
  h.step(320); assert.equal(h.drawCalls.at(-1)[0],h.images[0]);
  h.renderer.update(h.state(2), 2); h.step(354);
  assert.equal(h.drawCalls.at(-1)[0],h.images[0]);
  h.renderer.dispose(); assert.equal(h.scheduled,0);
});

test('a tray toggle and completed drag merge into settings without losing edits or rewriting placement', () => {
  const baseline = { keyboard: true, mouse: false, movement: false, top: true, scale: '100', monitor: 'primary',
    'position-x': '0', 'position-y': '0', 'idle-timeout': '250', 'rate-evaluation': '50', 'rate-window': '2000', 'rate-stability': '100' };
  const latest = { ...baseline, mouse: true, 'position-x': '300', 'position-y': '110' };
  const edited = mergeSettings({ ...baseline, keyboard: false }, baseline, latest);
  assert.deepEqual(edited, { ...latest, keyboard: false });
  const update = settingsUpdate(edited, latest);
  assert.equal(update.keyboard, false);
  assert.equal(update.mouse, null);
  assert.equal(update.basePlacement, null);
  assert.throws(() => settingsUpdate({ ...edited, 'idle-timeout': '' }, latest), /빈 값/);
  assert.throws(() => settingsUpdate({ ...edited, 'idle-timeout': '50', 'rate-evaluation': '60' }, latest), /평가 간격/);
});

test('pointer jitter does not start a drag; rapid motion is coalesced and cancellation never commits', async () => {
  let captured = false;
  const canvas = { setPointerCapture() { captured = true; }, hasPointerCapture() { return captured; }, releasePointerCapture() { captured = false; } };
  const calls = [], errors = [], gate = Promise.withResolvers(), sent = Promise.withResolvers();
  installCharacterDrag(canvas, async (name, args) => {
    calls.push([name, args]);
    if (name === 'begin_character_drag') return 'session';
    if (name === 'update_character_edit' && calls.length === 2) { sent.resolve(); await gate.promise; }
  }, error => errors.push(error));
  const event = (screenX) => ({ button: 0, pointerId: 1, screenX, screenY: 0 });
  canvas.onpointerdown(event(0)); canvas.onpointermove(event(2)); canvas.onpointerup(event(2));
  assert.equal(calls.length, 0);
  canvas.onpointerdown(event(0)); canvas.onpointermove(event(5));
  await sent.promise;
  for (let i = 6; i <= 50; i++) canvas.onpointermove(event(i));
  canvas.onpointerup(event(50)); gate.resolve();
  await new Promise(setImmediate);
  assert.deepEqual(calls, [['begin_character_drag', undefined], ['update_character_edit', { sessionId: 'session', dx: 5, dy: 0 }], ['update_character_edit', { sessionId: 'session', dx: 45, dy: 0 }], ['finish_character_edit', { sessionId: 'session' }]]);
  calls.length = 0;
  canvas.onpointerdown(event(0)); canvas.onpointermove(event(10)); canvas.onpointercancel(event(10));
  await new Promise(setImmediate);
  assert.deepEqual(calls.map(([name]) => name), ['begin_character_drag', 'cancel_character_edit']);
  assert.deepEqual(errors, []);
});
