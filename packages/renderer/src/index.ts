import type {
  Character,
  CharacterState,
  FrameAck,
  FrameOutcome,
  RenderPlan,
} from "../../contracts/generated/index";

/** The renderer owns drawing only. Rust owns action selection and completion. */
export class CharacterRenderer {
  private state: CharacterState | null = null;
  private receivedAt = 0;
  private frameRequest = 0;
  private frameTimer = 0;
  private lastPaint = -Infinity;
  private lastSignature = "";
  private images = new Map<
    string,
    { image: HTMLImageElement; ready: boolean; failed: boolean }
  >();
  private plan: RenderPlan;
  private visible = true;
  private seq = 0;
  private acknowledgedKey = "";
  private acknowledgedDrawn = false;
  private acknowledgedError = false;
  private context: CanvasRenderingContext2D;
  constructor(
    private canvas: HTMLCanvasElement,
    private character: Character,
    imageUrls: string | Record<string, string>,
    private onFrame: (ack: FrameAck) => void,
    plan?: RenderPlan,
  ) {
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Canvas 2D를 사용할 수 없습니다.");
    this.context = context;
    this.plan = plan ?? {
      animations: Object.fromEntries(
        Object.entries(character.animations)
          .filter(
            (entry): entry is [string, NonNullable<(typeof entry)[1]>] =>
              !!entry[1],
          )
          .map(([name, a]) => [
            name,
            {
              looping: a.loop,
              frames: a.frames.map((i) => ({
                image: "sheet",
                region: {
                  x:
                    (i % (character.sheet.width / character.sheet.frameWidth)) *
                    character.sheet.frameWidth,
                  y:
                    Math.floor(
                      i / (character.sheet.width / character.sheet.frameWidth),
                    ) * character.sheet.frameHeight,
                  width: character.sheet.frameWidth,
                  height: character.sheet.frameHeight,
                },
                durationMs: 1000 / a.fps,
              })),
            },
          ]),
      ),
    };
    for (const [id, url] of Object.entries(
      typeof imageUrls === "string" ? { sheet: imageUrls } : imageUrls,
    )) {
      const asset = { image: new Image(), ready: false, failed: false };
      asset.image.onload = () => {
        asset.ready = true;
        asset.failed = false;
        this.schedule();
      };
      asset.image.onerror = () => {
        asset.failed = true;
        this.schedule();
      };
      this.images.set(id, asset);
      asset.image.src = url;
    }
  }
  update(state: CharacterState, seq: number) {
    this.state = state;
    this.seq = seq;
    this.receivedAt = performance.now();
    this.stop();
    this.schedule();
  }
  resize(width: number, height: number) {
    const ratio = window.devicePixelRatio || 1;
    const pixelWidth = Math.max(1, Math.round(width * ratio)),
      pixelHeight = Math.max(1, Math.round(height * ratio));
    if (this.canvas.width === pixelWidth && this.canvas.height === pixelHeight)
      return;
    this.canvas.width = pixelWidth;
    this.canvas.height = pixelHeight;
    this.canvas.style.width = `${width}px`;
    this.canvas.style.height = `${height}px`;
    this.lastSignature = "";
    this.schedule();
  }
  setVisible(visible: boolean) {
    this.visible = visible;
    if (visible) this.schedule();
    else this.stop();
  }
  stop() {
    if (this.frameRequest) cancelAnimationFrame(this.frameRequest);
    if (this.frameTimer) clearTimeout(this.frameTimer);
    this.frameRequest = 0;
    this.frameTimer = 0;
  }
  dispose() {
    this.stop();
    this.visible = false;
    for (const { image } of this.images.values()) {
      image.onload = null;
      image.onerror = null;
    }
    this.images.clear();
  }
  private schedule(delay = 0) {
    if (!this.visible || this.frameRequest || this.frameTimer) return;
    if (delay > 0)
      this.frameTimer = window.setTimeout(() => {
        this.frameTimer = 0;
        this.schedule();
      }, delay);
    else this.frameRequest = requestAnimationFrame((time) => this.paint(time));
  }
  private paint(time: number) {
    this.frameRequest = 0;
    if (!this.state || !this.visible) return;
    const animation =
      this.plan.animations[this.state.animation] ?? this.plan.animations.idle;
    if (!animation?.frames.length) {
      this.drawError();
      return;
    }
    const elapsed =
      this.state.animationElapsedMs + Math.max(0, time - this.receivedAt);
    const duration = animation.frames.reduce((n, f) => n + f.durationMs, 0);
    let position = animation.looping
      ? elapsed % duration
      : Math.min(elapsed, duration - 0.001);
    let index = 0;
    while (
      index < animation.frames.length - 1 &&
      position >= animation.frames[index].durationMs
    )
      position -= animation.frames[index++].durationMs;
    const frame = animation.frames[index];
    const asset = this.images.get(frame.image);
    if (!asset || asset.failed) {
      this.drawError();
      return;
    }
    if (!asset.ready) return;
    const signature = `${this.state.epoch}:${this.state.actionInstanceId}:${this.state.animation}:${index}:${this.state.facingLeft}:${this.canvas.width}:${this.canvas.height}`;
    if (
      signature !== this.lastSignature &&
      time - this.lastPaint >= 1000 / 30
    ) {
      const anchor = this.character.anchor.x;
      const extent = Math.max(anchor, 1 - anchor);
      const viewportWidth = this.canvas.width / (2 * extent);
      const fit = Math.min(
        viewportWidth / frame.region.width,
        this.canvas.height / frame.region.height,
      );
      const destinationWidth = frame.region.width * fit,
        destinationHeight = frame.region.height * fit;
      const destinationX = this.canvas.width / 2 - anchor * destinationWidth;
      const destinationY =
        (this.canvas.height - destinationHeight) * this.character.anchor.y;
      this.context.setTransform(1, 0, 0, 1, 0, 0);
      this.context.clearRect(0, 0, this.canvas.width, this.canvas.height);
      this.context.imageSmoothingEnabled = false;
      if (this.state.facingLeft)
        this.context.setTransform(-1, 0, 0, 1, this.canvas.width, 0);
      try {
        this.context.drawImage(
          asset.image,
          frame.region.x,
          frame.region.y,
          frame.region.width,
          frame.region.height,
          destinationX,
          destinationY,
          destinationWidth,
          destinationHeight,
        );
      } catch {
        this.drawError();
        return;
      }
      this.lastPaint = time;
      this.lastSignature = signature;
      this.acknowledge("drawn");
    }
    if (signature !== this.lastSignature)
      this.schedule(Math.max(1, this.lastPaint + 1000 / 30 - time));
    else if (
      (animation.looping && animation.frames.length > 1) ||
      index < animation.frames.length - 1
    )
      this.schedule(Math.max(1, frame.durationMs - position));
  }
  private drawError() {
    if (!this.state || !this.visible) return;
    this.context.fillStyle = "#f7bb82";
    this.context.fillRect(8, 8, 64, 64);
    this.context.fillStyle = "#302f43";
    this.context.font = "20px sans-serif";
    this.context.fillText("!", 34, 48);
    this.acknowledge("error");
  }
  private acknowledge(outcome: FrameOutcome) {
    if (!this.state) return;
    const key = `${this.state.epoch}:${this.state.actionInstanceId}`;
    if (key !== this.acknowledgedKey) {
      this.acknowledgedKey = key;
      this.acknowledgedDrawn = false;
      this.acknowledgedError = false;
    }
    if (outcome === "drawn") {
      if (this.acknowledgedDrawn) return;
      this.acknowledgedDrawn = true;
    } else {
      if (this.acknowledgedError) return;
      this.acknowledgedError = true;
    }
    this.onFrame({
      seq: this.seq,
      epoch: this.state.epoch,
      actionInstanceId: this.state.actionInstanceId,
      outcome,
    });
  }
}
