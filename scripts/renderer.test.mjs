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

async function transpileModule(name, directory = '../apps/player/src') {
  const source = await readFile(new URL(`${directory}/${name}.ts`, import.meta.url), 'utf8');
  const file = join(temp, `${name}.mjs`);
  await writeFile(file, ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } }).outputText);
  return import(pathToFileURL(file));
}
const { mergeSettings, settingsUpdate } = await transpileModule('settings-model');
const { installCharacterDrag } = await transpileModule('drag');
const { trackCharacterPixels } = await transpileModule('hit-mask');
const { ReactionAudio } = await transpileModule('audio', '../packages/renderer/src');

test('prepared event sounds start from a decoded buffer and do not reopen the file', async t => {
  const previous = globalThis.AudioContext;
  const calls = [];
  globalThis.AudioContext = class {
    state = 'running';
    async decodeAudioData(bytes) { calls.push(['decode', bytes.byteLength]); return { length: 256, numberOfChannels: 1 }; }
    createGain() { return { gain: { value: 1 }, connect() {}, disconnect() {} }; }
    createBufferSource() {
      return { connect: gain => gain, disconnect() {}, start() { calls.push(['start']); }, stop() { calls.push(['stop']); } };
    }
  };
  t.after(() => { if (previous) globalThis.AudioContext = previous; else delete globalThis.AudioContext; });
  const audio = new ReactionAudio(() => '/cue.wav', assert.fail, async () => new ArrayBuffer(128));
  await audio.preload(['cue']);
  assert.deepEqual(calls, [['decode', 128]]);
  assert.equal(audio.waitForCue('cue'), undefined);
  const state = { epoch: 1, actionInstanceId: 2, sound: { resource: 'cue', volume: 0.7 } };
  audio.update(state);
  audio.update(state);
  audio.stop();
  assert.deepEqual(calls, [['decode', 128], ['start'], ['stop']]);
});

test('reaction audio plays once per action, stops on replacement or hiding, and plays on a restart', t => {
  const originalAudio = globalThis.Audio;
  const calls = [];
  globalThis.Audio = class {
    constructor(url) { this.url = url; }
    async play() { calls.push(['play', this.url, this.volume]); }
    pause() { calls.push(['pause', this.url]); }
  };
  t.after(() => {
    if (originalAudio) globalThis.Audio = originalAudio;
    else delete globalThis.Audio;
  });
  const audio = new ReactionAudio(resource => `/${resource}.wav`, error => assert.fail(error));
  const state = { epoch: 1, actionInstanceId: 1, sound: { resource: 'ping', volume: 0.65 } };
  audio.update(state);
  audio.update({ ...state, facingLeft: true });
  audio.update({ ...state, actionInstanceId: 2, sound: { resource: 'pop', volume: 1 } });
  audio.update({ ...state, actionInstanceId: 2 }, false);
  audio.update({ ...state, actionInstanceId: 2 }, true);
  audio.update({ ...state, actionInstanceId: 3 });
  assert.deepEqual(calls, [
    ['play', '/ping.wav', 0.65],
    ['pause', '/ping.wav'],
    ['play', '/pop.wav', 1],
    ['pause', '/pop.wav'],
    ['play', '/ping.wav', 0.65],
  ]);
});

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
  const renderer = new CharacterRenderer(canvas, character, options.images ?? 'test.png', ack => acknowledgements.push(ack), options.plan, options.drawFirstFrameImmediately);
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
  h.renderer.update(h.state(1), 1); h.step(0); h.images[0].onload(); h.step(0);
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

test('clip scaling reserves room while keeping unscaled clips at their original size', t => {
  const h = harness(t);
  h.renderer.resize(384, 384);
  h.renderer.setAnimationLayout({ jump: { scalePercent: 150 } }, 1.5);
  h.renderer.update(h.state(1), 1);
  h.step(0); h.images[0].onload(); h.step(0);
  assert.deepEqual(h.drawCalls.at(-1).slice(5), [64, 128, 256, 256]);

  h.renderer.update({ ...h.state(2), animation: 'jump' }, 2);
  h.step(50);
  assert.deepEqual(h.drawCalls.at(-1).slice(5), [0, 0, 384, 384]);

  h.renderer.setAnimationLayout({ jump: { scalePercent: 125 } }, 1.5);
  h.step(100);
  assert.deepEqual(h.drawCalls.at(-1).slice(5), [32, 64, 320, 320]);
});

test('clip anchors line up at the same canvas position in both orientations', t => {
  const h = harness(t);
  const transforms = [];
  h.context.setTransform = (...args) => transforms.push(args);
  h.renderer.resize(384, 256);
  h.renderer.setAnimationLayout({ jump: { anchorX: 0.25 } }, 1);
  h.renderer.update(h.state(1), 1);
  h.step(0); h.images[0].onload(); h.step(0);
  assert.deepEqual(h.drawCalls.at(-1).slice(5), [64, 0, 256, 256]);

  h.renderer.update({ ...h.state(2), animation: 'jump' }, 2);
  h.step(50);
  assert.deepEqual(h.drawCalls.at(-1).slice(5), [128, 0, 256, 256]);
  assert.deepEqual(transforms.at(-1), [1, 0, 0, 1, 0, 0]);

  h.renderer.update({ ...h.state(2), animation: 'jump', facingLeft: true }, 3);
  h.step(100);
  assert.deepEqual(h.drawCalls.at(-1).slice(5), [128, 0, 256, 256]);
  assert.deepEqual(transforms.at(-1), [-1, 0, 0, 1, 384, 0]);
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

test('the first overlay frame can draw before the native window enables animation frames', t => {
  const h = harness(t, { drawFirstFrameImmediately: true });
  h.renderer.update(h.state(1), 10);
  h.renderer.update(h.state(2), 11);
  assert.equal(h.acknowledgements.length, 0);
  assert.equal(h.scheduled, 1); // watchdog while the first image is loading
  h.images[0].onload();
  assert.deepEqual(h.acknowledgements.map(ack => [ack.seq, ack.outcome]), [[11, 'drawn']]);
  h.renderer.update(h.state(3), 12);
  assert.equal(h.draws, 1);
  h.step(34);
  assert.equal(h.draws, 2);
});

test('hidden states do not draw or acknowledge, and resuming uses the latest state', t => {
  const h = harness(t);
  h.renderer.setVisible(false);
  h.renderer.update(h.state(1), 1); h.step(50);
  h.renderer.update(h.state(2), 2); h.step(100);
  assert.equal(h.draws, 0); assert.equal(h.acknowledgements.length, 0); assert.equal(h.scheduled, 0);
  h.renderer.setVisible(true); h.step(150); h.images[0].onload(); h.step(150);
  assert.equal(h.acknowledgements[0].actionInstanceId, 2);
});

test('a failed image is fetched again and a stalled request also recovers without reloading', t => {
  const h = harness(t);
  h.renderer.update(h.state(1), 1); h.step(0);
  h.images[0].onerror(); h.step(10);
  h.renderer.resize(512, 512); h.step(20);
  assert.deepEqual(h.acknowledgements.map(ack => ack.outcome), ['error']);
  assert.equal(h.draws, 0);
  h.step(99);
  assert.equal(h.images.length, 1);
  h.step(100);
  assert.match(h.images[1].src, /^test\.png\?buddy-retry=/);
  h.step(5100);
  h.step(5400);
  assert.notEqual(h.images[1].src, h.images[2].src);
  assert.equal(h.images[1].onload, null);
  h.images[2].onload(); h.step(5400);
  assert.deepEqual(h.acknowledgements.map(ack => ack.outcome), ['error', 'drawn']);
  assert.equal(h.draws, 1);
  h.context.fail = true; h.renderer.update(h.state(2), 2); h.step(5440);
  assert.equal(h.acknowledgements.at(-1).outcome, 'error');
  assert.equal(h.acknowledgements.at(-1).actionInstanceId, 2);
  h.context.fail = false; h.step(5540); h.images[3].onload(); h.step(5540);
  assert.equal(h.acknowledgements.at(-1).outcome, 'drawn');
});

test('a failed intermediate frame does not stop later frames or cover the last good image', t => {
  const region = { x: 0, y: 0, width: 30, height: 30 };
  const plan = { animations: { idle: { looping: true,
    frames: ['first', 'missing', 'last'].map(image => ({ image, region, durationMs: 100 })),
  } } };
  const h = harness(t, { images: { first: 'first.png', missing: 'missing.png', last: 'last.png', unused: 'unused.png' }, plan });
  h.renderer.update(h.state(1), 1); h.step(0);
  h.images[0].onload(); h.images[1].onerror(); h.images[2].onload();
  h.step(0);
  h.step(100);
  assert.equal(h.draws, 1);
  assert.equal(h.errors, 0);
  h.step(200);
  assert.equal(h.drawCalls.at(-1)[0], h.images[2]);
  assert.ok(h.images.every(image => image.src !== 'unused.png'));
});

test('prepared GIF timing, resource switching, crop coordinates and restart use the state clock', t => {
  const frame = (image, x, durationMs) => ({ image, region: { x, y: 3, width: 20, height: 30 }, durationMs });
  const h = harness(t, { images: { first: 'first.png', second: 'second.png' }, plan: { animations: { idle: { looping: true, frames: [frame('first', 10, 80), frame('second', 25, 240)] } } } });
  h.renderer.update(h.state(1), 1); h.step(0);
  h.images.forEach(image => image.onload()); h.step(0);
  assert.equal(h.drawCalls[0][0],h.images[0]); assert.deepEqual(h.drawCalls[0].slice(1,5),[10,3,20,30]);
  h.step(79); assert.equal(h.draws,1);
  h.step(80); assert.equal(h.drawCalls.at(-1)[0],h.images[1]);
  assert.deepEqual(h.drawCalls.at(-1).slice(1,5),[25,3,20,30]);
  h.step(320); assert.equal(h.drawCalls.at(-1)[0],h.images[0]);
  h.renderer.update(h.state(2), 2); h.step(354);
  assert.equal(h.drawCalls.at(-1)[0],h.images[0]);
  h.renderer.dispose(); assert.equal(h.scheduled,0);
});

test('sprite regions of different sizes keep one scale and baseline throughout a clip', t => {
  const frames = [32, 16].map(width => ({
    image: 'sheet', region: { x: 0, y: 0, width, height: 16 }, durationMs: 80,
  }));
  const h = harness(t, { plan: { animations: { idle: { looping: true, frames } } } });
  h.renderer.update(h.state(1), 1); h.step(0);
  h.images[0].onload(); h.step(0);
  h.step(80);
  assert.deepEqual(h.drawCalls.map(call => call.slice(5)), [
    [0, 128, 256, 128],
    [64, 128, 128, 128],
  ]);
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

test('a short tap clicks once while drag, capture loss and cancellation never click', async () => {
  let captured = false;
  const canvas = { getBoundingClientRect: () => ({ left: -5, top: -10 }), setPointerCapture() { captured = true; }, hasPointerCapture() { return captured; }, releasePointerCapture() { captured = false; } };
  const calls = [], errors = [], gate = Promise.withResolvers(), sent = Promise.withResolvers();
  installCharacterDrag(canvas, async (name, args) => {
    calls.push([name, args]);
    if (name === 'begin_character_drag') return 'session';
    if (name === 'update_character_edit' && calls.length === 2) { sent.resolve(); await gate.promise; }
  }, error => errors.push(error), event => event.clientX > 0);
  const event = (screenX) => ({ button: 0, pointerId: 1, screenX, screenY: 0, clientX: 12, clientY: 18 });
  canvas.onpointerdown({ ...event(0), clientX: 0 }); canvas.onpointerup(event(0));
  assert.equal(captured, false);
  canvas.onpointerdown(event(0)); canvas.onpointermove(event(2)); canvas.onpointerup(event(2));
  assert.equal(calls.length, 0);
  await new Promise(setImmediate);
  assert.deepEqual(calls, [['character_click', undefined]]);
  calls.length = 0;
  canvas.onpointerdown(event(0)); canvas.onlostpointercapture(event(1)); canvas.onpointerup(event(1));
  await new Promise(setImmediate);
  assert.deepEqual(calls, []);
  canvas.onpointerdown(event(0)); canvas.onpointermove(event(5));
  await sent.promise;
  for (let i = 6; i <= 50; i++) canvas.onpointermove(event(i));
  canvas.onpointerup(event(50)); gate.resolve();
  await new Promise(setImmediate);
  assert.deepEqual(calls, [['begin_character_drag', { grab: { x: 17, y: 28 } }], ['update_character_edit', { sessionId: 'session' }], ['update_character_edit', { sessionId: 'session' }], ['finish_character_edit', { sessionId: 'session' }]]);
  calls.length = 0;
  canvas.onpointerdown(event(0)); canvas.onpointermove(event(5));
  await new Promise(setImmediate);
  canvas.onpointerup(event(10));
  await new Promise(setImmediate);
  assert.deepEqual(calls.map(([name]) => name), ['begin_character_drag', 'update_character_edit', 'update_character_edit', 'finish_character_edit']);
  calls.length = 0;
  canvas.onpointerdown(event(0)); canvas.onpointermove(event(10)); canvas.onpointercancel(event(10));
  await new Promise(setImmediate);
  assert.deepEqual(calls.map(([name]) => name), ['begin_character_drag', 'cancel_character_edit']);
  calls.length = 0;
  canvas.onpointerdown(event(0)); canvas.onpointerup(event(8));
  await new Promise(setImmediate);
  assert.deepEqual(calls.map(([name]) => name), ['begin_character_drag', 'update_character_edit', 'finish_character_edit']);
  assert.deepEqual(errors, []);
});

test('the hit mask follows visible pixels, skips transparent holes and clears on resize', async t => {
  const originalDocument = globalThis.document, originalWindow = globalThis.window;
  const data = new Uint8ClampedArray(9 * 4);
  const context = { clearRect() {}, drawImage() {}, getImageData: () => ({ data }) };
  globalThis.document = { createElement: () => ({ getContext: () => context }) };
  globalThis.window = { devicePixelRatio: 2 };
  t.after(() => { globalThis.document = originalDocument; globalThis.window = originalWindow; });
  const canvas = { width: 6, height: 6, getBoundingClientRect: () => ({ left: 2, top: 3, width: 3, height: 3 }) };
  const masks = [];
  const tracker = trackCharacterPixels(canvas, async mask => { masks.push(mask); }, assert.fail);
  data[3] = data[7] = data[35] = 255;
  data[19] = 10;
  tracker.update(true);
  assert.equal(tracker.contains({ clientX: 2.5, clientY: 3.5 }), true);
  assert.equal(tracker.contains({ clientX: 3.5, clientY: 4.5 }), false);
  assert.equal(tracker.contains({ clientX: 5, clientY: 5.5 }), false);
  tracker.update(true);
  await new Promise(setImmediate);
  assert.deepEqual(masks, [{ columns: 3, rows: 3, bits: [259], offsetX: 0, offsetY: 0 }]);
  tracker.update(false);
  assert.equal(tracker.contains({ clientX: 2.5, clientY: 3.5 }), false);
  assert.deepEqual(masks.at(-1), { columns: 0, rows: 0, bits: [], offsetX: 0, offsetY: 0 });
});
