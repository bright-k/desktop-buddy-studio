import type { CharacterState } from "../../contracts/generated/index";

/** A reaction can publish many snapshots while it plays a single sound. */
export class ReactionAudio {
  private action = "";
  private playing: HTMLAudioElement | null = null;
  private context?: AudioContext;
  private buffers = new Map<string, AudioBuffer>();
  private loading = new Map<string, { done: Promise<void>; deadline: number }>();
  private unavailable = new Set<string>();
  private stopBuffer?: () => void;
  private decodedBytes = 0;

  constructor(
    private resolve: (resource: string) => string | undefined,
    private onError: (error: unknown) => void,
    private readBytes?: (resource: string) => Promise<ArrayBuffer>,
  ) {}

  async preload(resources: string[]) {
    for (const id of resources) await this.prepare(id)?.done;
  }

  activate() {
    if (this.context?.state === "suspended") void this.context.resume().catch(() => {});
  }

  /** Bound the wait for a first use while the background decode is still running. */
  waitForCue(resource: string): Promise<void> | undefined {
    const loading = this.prepare(resource);
    if (!loading) return;
    const delay = loading.deadline - performance.now();
    if (delay <= 0) return;
    return new Promise<void>((resolve) => {
      const timer = setTimeout(resolve, delay);
      void loading.done.then(() => { clearTimeout(timer); resolve(); });
    });
  }

  private prepare(resource: string) {
    const readBytes = this.readBytes;
    if (!readBytes || this.buffers.has(resource) || this.unavailable.has(resource)) return;
    if (!this.context) {
      if (typeof AudioContext !== "function") return;
      try { this.context = new AudioContext(); this.activate(); } catch { return; }
    }
    const context = this.context;
    const existing = this.loading.get(resource);
    if (existing) return existing;
    const done = (async () => {
      const buffer = await context.decodeAudioData(await readBytes(resource));
      const bytes = buffer.length * buffer.numberOfChannels * 4;
      if (this.decodedBytes + bytes > 64 * 1024 * 1024) throw new Error("Audio cache is full");
      this.decodedBytes += bytes;
      this.buffers.set(resource, buffer);
    })().catch(() => { this.unavailable.add(resource); }).finally(() => this.loading.delete(resource));
    const load = { done, deadline: performance.now() + 450 };
    this.loading.set(resource, load);
    return load;
  }

  update(state: CharacterState, visible = true) {
    const action = `${state.epoch}:${state.actionInstanceId}`;
    if (!visible) this.stop();
    if (action === this.action) return;
    this.action = action;
    this.stop();
    if (!visible || !state.sound || state.sound.volume === 0) return;
    const url = this.resolve(state.sound.resource);
    if (!url) return;
    const context = this.context;
    const buffer = this.buffers.get(state.sound.resource);
    if (context?.state === "running" && buffer) {
      const source = context.createBufferSource();
      const gain = context.createGain();
      source.buffer = buffer;
      gain.gain.value = state.sound.volume;
      source.connect(gain).connect(context.destination);
      const disconnect = () => { source.disconnect(); gain.disconnect(); };
      const stop = () => { source.stop(); disconnect(); };
      this.stopBuffer = stop;
      source.onended = () => {
        disconnect();
        if (this.stopBuffer === stop) this.stopBuffer = undefined;
      };
      source.start();
      return;
    }
    const audio = new Audio(url);
    audio.volume = state.sound.volume;
    this.playing = audio;
    void audio.play().catch((error: unknown) => {
      if (this.playing === audio) this.onError(error);
    });
  }

  stop() {
    this.stopBuffer?.();
    this.stopBuffer = undefined;
    this.playing?.pause();
    this.playing = null;
  }
}
