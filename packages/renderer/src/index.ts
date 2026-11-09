import type {
  Character,
  CharacterState,
  FrameAck,
  FrameOutcome,
  RenderAnimation,
  RenderPlan,
} from "../../contracts/generated/index";

type ImageAsset = {
  url: string;
  image?: HTMLImageElement;
  status: "new" | "loading" | "ready" | "failed";
  failures: number;
  deadline: number;
};

const IMAGE_TIMEOUT_MS = 5000;
const RETRY_DELAYS_MS = [100, 300, 1000, 3000, 10000];

function scaleEnvelope(animations: Character["animations"]): number {
  return Math.max(100, ...Object.values(animations).map(clip => clip?.scalePercent ?? 100)) / 100;
}

export function horizontalAnchorExtent(character: Pick<Character, "anchor" | "animations">): number {
  const x = character.anchor.x;
  let extent = Math.max(x, 1 - x);
  for (const animation of Object.values(character.animations)) {
    const anchor = animation?.anchorX ?? x;
    extent = Math.max(extent, anchor, 1 - anchor);
  }
  return extent;
}

/** The renderer owns drawing only. Rust owns action selection and completion. */
export class CharacterRenderer {
  private state: CharacterState | null = null;
  private receivedAt = 0;
  private frameRequest = 0;
  private frameTimer = 0;
  private lastPaint = -Infinity;
  private lastSignature = "";
  private images = new Map<string, ImageAsset>();
  private currentImage?: ImageAsset;
  private extents = new Map<RenderAnimation, { width: number; height: number; duration: number }>();
  private plan: RenderPlan;
  private layoutAnimations: Character["animations"];
  private reservedScale: number;
  private horizontalExtent: number;
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
    private drawFirstFrameImmediately = false,
    private onPixelsChanged?: (drawn: boolean) => void,
  ) {
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Canvas 2D를 사용할 수 없습니다.");
    this.context = context;
    this.layoutAnimations = character.animations;
    this.reservedScale = scaleEnvelope(this.layoutAnimations);
    this.horizontalExtent = horizontalAnchorExtent(character);
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
    for (const animation of Object.values(this.plan.animations)) {
      if (!animation) continue;
      const extent = { width: 0, height: 0, duration: 0 };
      for (const frame of animation.frames) {
        extent.width = Math.max(extent.width, frame.region.width);
        extent.height = Math.max(extent.height, frame.region.height);
        extent.duration += frame.durationMs;
      }
      this.extents.set(animation, extent);
    }
    for (const [id, url] of Object.entries(
      typeof imageUrls === "string" ? { sheet: imageUrls } : imageUrls,
    )) {
      this.images.set(id, { url, status: "new", failures: 0, deadline: 0 });
    }
  }
  update(state: CharacterState, seq: number) {
    this.state = state;
    this.seq = seq;
    this.receivedAt = performance.now();
    this.currentImage = undefined;
    this.stop();
    this.schedule();
  }
  setAnimationLayout(
    animations: Character["animations"],
    reservedScale: number,
    horizontalExtent = horizontalAnchorExtent({ anchor: this.character.anchor, animations }),
  ) {
    this.layoutAnimations = animations;
    this.reservedScale = reservedScale;
    this.horizontalExtent = horizontalExtent;
    this.lastSignature = "";
    this.stop();
    this.schedule();
  }
  resize(width: number, height: number) {
    const ratio = window.devicePixelRatio || 1;
    const pixelWidth = Math.max(1, Math.round(width * ratio)),
      pixelHeight = Math.max(1, Math.round(height * ratio));
    this.canvas.style.width = `${width}px`;
    this.canvas.style.height = `${height}px`;
    if (this.canvas.width === pixelWidth && this.canvas.height === pixelHeight)
      return;
    this.canvas.width = pixelWidth;
    this.canvas.height = pixelHeight;
    this.lastSignature = "";
    this.onPixelsChanged?.(false);
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
    for (const asset of this.images.values()) this.releaseImage(asset);
    this.images.clear();
    this.currentImage = undefined;
  }
  private releaseImage(asset: ImageAsset) {
    if (asset.image) {
      asset.image.onload = null;
      asset.image.onerror = null;
      asset.image = undefined;
    }
  }
  private failImage(asset: ImageAsset) {
    this.releaseImage(asset);
    asset.status = "failed";
    asset.deadline = performance.now() + RETRY_DELAYS_MS[
      Math.min(asset.failures++, RETRY_DELAYS_MS.length - 1)
    ];
  }
  private prepareImage(asset: ImageAsset | undefined, now: number) {
    if (!asset) return;
    if (asset.status === "loading" && now >= asset.deadline)
      this.failImage(asset);
    if (asset.status !== "new" && !(asset.status === "failed" && now >= asset.deadline))
      return;
    const image = new Image();
    // The overlay reads the composited alpha to decide which pixels can be clicked.
    if (this.onPixelsChanged) image.crossOrigin = "anonymous";
    asset.image = image;
    asset.status = "loading";
    asset.deadline = now + IMAGE_TIMEOUT_MS;
    const wake = () => {
      if (this.currentImage !== asset) return;
      this.stop();
      this.schedule();
    };
    image.onload = () => {
      asset.status = "ready";
      wake();
    };
    image.onerror = () => {
      this.failImage(asset);
      wake();
    };
    // Bypass a cached failed response without changing the asset path.
    const [base, fragment] = asset.url.split(/#(.*)/s);
    image.src = asset.failures && !/^(data|blob):/.test(base)
      ? `${base}${base.includes("?") ? "&" : "?"}buddy-retry=${Date.now()}-${asset.failures}${fragment ? "#" + fragment : ""}`
      : asset.url;
  }
  private schedule(delay = 0) {
    if (!this.visible || !this.state || this.frameRequest || this.frameTimer) return;
    if (delay > 0)
      this.frameTimer = window.setTimeout(() => {
        this.frameTimer = 0;
        this.schedule();
      }, delay);
    // A hidden native window may not receive animation frame callbacks yet.
    else if (this.drawFirstFrameImmediately) this.paint(performance.now());
    else this.frameRequest = requestAnimationFrame((time) => this.paint(time));
  }
  private paint(time: number) {
    this.frameRequest = 0;
    if (!this.state || !this.visible) return;
    const name = this.plan.animations[this.state.animation] ? this.state.animation : "idle";
    const animation = this.plan.animations[name];
    if (!animation?.frames.length) {
      this.drawError();
      return;
    }
    const elapsed =
      this.state.animationElapsedMs + Math.max(0, time - this.receivedAt);
    const { duration, width, height } = this.extents.get(animation)!;
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
    this.currentImage = asset;
    const now = performance.now();
    this.prepareImage(asset, now);
    // Decode only the current clip's nearby frames, rather than opening every
    // image in the project when the Player starts or Studio changes tabs.
    for (let offset = 1; offset <= 3 && offset < animation.frames.length; offset++) {
      const next = animation.frames[index + offset]
        ?? (animation.looping ? animation.frames[(index + offset) % animation.frames.length] : undefined);
      if (next) this.prepareImage(this.images.get(next.image), now);
    }
    const nextFrame = (animation.looping && animation.frames.length > 1)
      || index < animation.frames.length - 1
      ? Math.max(1, frame.durationMs - position)
      : Infinity;
    if (asset?.status !== "ready" || !asset.image) {
      if (!asset || asset.failures) this.drawError();
      const delay = Math.min(nextFrame, asset ? Math.max(1, asset.deadline - now) : Infinity);
      if (Number.isFinite(delay)) this.schedule(delay);
      return;
    }
    const signature = `${this.state.epoch}:${this.state.actionInstanceId}:${this.state.animation}:${index}:${this.state.facingLeft}:${this.canvas.width}:${this.canvas.height}`;
    if (
      signature !== this.lastSignature &&
      time - this.lastPaint >= 1000 / 30
    ) {
      const clip = this.layoutAnimations[name];
      const anchor = clip?.anchorX ?? this.character.anchor.x;
      const viewportWidth = this.canvas.width / (2 * this.horizontalExtent);
      const fit = Math.min(
        viewportWidth / width,
        this.canvas.height / height,
      ) * ((clip?.scalePercent ?? 100) / 100) / this.reservedScale;
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
        this.lastSignature = "";
        this.failImage(asset);
        this.drawError();
        this.schedule(Math.min(nextFrame, Math.max(1, asset.deadline - now)));
        return;
      }
      asset.failures = 0;
      this.lastPaint = time;
      this.lastSignature = signature;
      this.onPixelsChanged?.(true);
      this.acknowledge("drawn");
    }
    if (signature !== this.lastSignature)
      this.schedule(Math.max(1, this.lastPaint + 1000 / 30 - time));
    else if (Number.isFinite(nextFrame)) this.schedule(nextFrame);
  }
  private drawError() {
    if (!this.state || !this.visible) return;
    if (!this.lastSignature) {
      this.context.setTransform(1, 0, 0, 1, 0, 0);
      this.context.fillStyle = "#f7bb82";
      this.context.fillRect(8, 8, 64, 64);
      this.context.fillStyle = "#302f43";
      this.context.font = "20px sans-serif";
      this.context.fillText("!", 34, 48);
      this.onPixelsChanged?.(false);
    }
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
    this.drawFirstFrameImmediately = false;
    this.onFrame({
      seq: this.seq,
      epoch: this.state.epoch,
      actionInstanceId: this.state.actionInstanceId,
      outcome,
    });
  }
}
