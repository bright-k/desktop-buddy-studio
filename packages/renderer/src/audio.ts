import type { CharacterState } from "../../contracts/generated/index";

const AUDIO_RECOVERY_MS = 150;
const AUDIO_CACHE_BYTES = 16 * 1024 * 1024;
const AUDIO_CUE_BYTES = 8 * 1024 * 1024;
const AUDIO_WARM_CUES = 2;

/** A reaction can publish many snapshots while it plays a single sound. */
export class ReactionAudio {
  private enabled = true;
  private enableRevision = 0;
  private action = "";
  private playing: HTMLAudioElement | null = null;
  private context?: AudioContext;
  private resuming?: Promise<void>;
  private buffers = new Map<string, AudioBuffer>();
  private loading?: { resource: string; context: AudioContext; done: Promise<void> };
  private unavailable = new Set<string>();
  private stopBuffer?: () => void;
  private playingResource?: string;
  private decodedBytes = 0;

  constructor(
    private resolve: (resource: string) => string | undefined,
    private onError: (error: unknown) => void,
    private readBytes?: (resource: string, sampleRate: number, maxDecodedBytes: number) => Promise<ArrayBuffer>,
  ) {}

  async preload(resources: string[]) {
    const revision = this.enableRevision;
    for (const id of resources.slice(0, AUDIO_WARM_CUES)) {
      if (this.loading) await this.loading.done;
      if (!this.enabled || this.enableRevision !== revision) return;
      await this.prepare(id);
    }
  }

  setEnabled(enabled: boolean) {
    if (this.enabled === enabled) return;
    this.enabled = enabled;
    this.enableRevision++;
    if (enabled) return;
    this.stop();
    const context = this.context;
    this.context = undefined;
    this.resuming = undefined;
    this.buffers.clear();
    this.unavailable.clear();
    this.decodedBytes = 0;
    if (context && context.state !== "closed") void context.close().catch(() => {});
  }

  dispose() {
    this.setEnabled(false);
  }

  activate() {
    const context = this.context;
    if (!context || context.state === "running" || context.state === "closed") return;
    this.resuming ??= new Promise<void>((resolve) => {
      // WebKit can leave resume pending when returning from the background.
      const timeout = setTimeout(resolve, AUDIO_RECOVERY_MS);
      void context.resume().catch(() => {}).then(() => {
        clearTimeout(timeout);
        resolve();
      });
    }).finally(() => { if (this.context === context) this.resuming = undefined; });
    return this.resuming;
  }

  /** Warm a candidate without delaying the first rendered frame. */
  prepare(resource: string): Promise<void> | undefined {
    const readBytes = this.readBytes;
    if (!this.enabled || !readBytes || this.buffers.has(resource) || this.unavailable.has(resource)) return;
    if (this.loading) return this.loading.resource === resource && this.loading.context === this.context
      ? this.loading.done : undefined;
    if (!this.context) {
      if (typeof AudioContext !== "function") return;
      try { this.context = new AudioContext(); this.activate(); } catch { return; }
    }
    const context = this.context;
    const done = (async () => {
      const source = await readBytes(resource, Math.ceil(context.sampleRate), AUDIO_CUE_BYTES);
      if (this.context !== context) return;
      if (!source.byteLength) {
        this.unavailable.add(resource);
        return;
      }
      const buffer = await context.decodeAudioData(source).catch(() => undefined);
      if (this.context !== context) return;
      if (!buffer) {
        this.unavailable.add(resource);
        return;
      }
      const bytes = buffer.length * buffer.numberOfChannels * 4;
      if (bytes > AUDIO_CUE_BYTES) {
        this.unavailable.add(resource);
        return;
      }
      for (const [id, cached] of this.buffers) {
        if (this.decodedBytes + bytes <= AUDIO_CACHE_BYTES) break;
        if (id === this.playingResource) continue;
        this.buffers.delete(id);
        this.decodedBytes -= cached.length * cached.numberOfChannels * 4;
      }
      this.decodedBytes += bytes;
      this.buffers.set(resource, buffer);
    })().catch(() => {})
      .finally(() => { this.loading = undefined; });
    this.loading = { resource, done, context };
    return done;
  }

  update(state: CharacterState, visible = true) {
    if (!visible) {
      this.stop();
      return;
    }
    const action = `${state.epoch}:${state.actionInstanceId}`;
    if (action === this.action) return;
    this.action = action;
    this.stop();
    const cue = state.sound;
    if (!this.enabled || !cue || cue.volume === 0) return;
    const url = this.resolve(cue.resource);
    if (!url) return;
    const context = this.context;
    const buffer = this.buffers.get(cue.resource);
    if (buffer) {
      this.buffers.delete(cue.resource);
      this.buffers.set(cue.resource, buffer);
    }
    if (context?.state === "running" && buffer && !this.resuming) {
      this.playingResource = cue.resource;
      this.playBuffer(context, buffer, cue.volume, url);
    } else {
      // A cue can end before WebKit resumes its audio context. Play it now
      // and let the context recover for subsequent reactions.
      if (buffer) void this.activate();
      this.playFile(url, cue.volume);
    }
  }

  private playBuffer(context: AudioContext, buffer: AudioBuffer, volume: number, url: string) {
    const source = context.createBufferSource();
    const gain = context.createGain();
    source.buffer = buffer;
    gain.gain.value = volume;
    source.connect(gain).connect(context.destination);
    const startedAt = context.currentTime;
    const watch = setTimeout(() => {
      // A resumed context may report running while its audio clock is stalled.
      if (this.stopBuffer === stop && context.currentTime <= startedAt) {
        this.stop();
        this.playFile(url, volume);
      }
    }, AUDIO_RECOVERY_MS);
    const disconnect = () => {
      clearTimeout(watch);
      source.onended = null;
      source.disconnect();
      gain.disconnect();
    };
    const stop = () => { source.stop(); disconnect(); };
    this.stopBuffer = stop;
    source.onended = () => {
      disconnect();
      if (this.stopBuffer === stop) {
        this.stopBuffer = undefined;
        this.playingResource = undefined;
      }
    };
    source.start();
  }

  private playFile(url: string, volume: number) {
    const audio = new Audio(url);
    audio.volume = volume;
    this.playing = audio;
    audio.onended = () => { if (this.playing === audio) this.playing = null; };
    void audio.play().catch((error: unknown) => {
      if (this.playing === audio) this.onError(error);
    });
  }

  stop() {
    this.stopBuffer?.();
    this.stopBuffer = undefined;
    this.playingResource = undefined;
    if (this.playing) {
      this.playing.onended = null;
      this.playing.pause();
      this.playing.removeAttribute?.("src");
      this.playing.load?.();
    }
    this.playing = null;
  }
}
