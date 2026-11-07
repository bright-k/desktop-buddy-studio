import type { CharacterState } from "../../contracts/generated/index";

/** A reaction can publish many snapshots while it plays a single sound. */
export class ReactionAudio {
  private action = "";
  private playing: HTMLAudioElement | null = null;

  constructor(
    private resolve: (resource: string) => string | undefined,
    private onError: (error: unknown) => void,
  ) {}

  update(state: CharacterState, visible = true) {
    const action = `${state.epoch}:${state.actionInstanceId}`;
    if (!visible) this.stop();
    if (action === this.action) return;
    this.action = action;
    this.stop();
    if (!visible || !state.sound || state.sound.volume === 0) return;
    const url = this.resolve(state.sound.resource);
    if (!url) return;
    const audio = new Audio(url);
    audio.volume = state.sound.volume;
    this.playing = audio;
    void audio.play().catch((error: unknown) => {
      if (this.playing === audio) this.onError(error);
    });
  }

  stop() {
    this.playing?.pause();
    this.playing = null;
  }
}
