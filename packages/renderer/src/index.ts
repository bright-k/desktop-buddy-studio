import type {
  Character,
  CharacterState,
  FrameAck,
  FrameOutcome,
  RenderAnimation,
  RenderPlan,
} from "../../contracts/generated/index";

type ImageAsset = {
  id: string;
  url: string;
  image?: HTMLImageElement;
  bitmap?: ImageBitmap;
  revision: number;
  status: "new" | "loading" | "ready" | "failed";
  failures: number;
  deadline: number;
  bytes: number;
};

const IMAGE_TIMEOUT_MS = 5000;
const RETRY_DELAYS_MS = [100, 300, 1000, 3000, 10000];
const IMAGE_CACHE_BYTES = 24 * 1024 * 1024;
const IMAGE_LOAD_LIMIT = 4;

function scaleEnvelope(animations: Character["animations"]): number {
  return Math.max(100, ...Object.values(animations).map(clip => clip?.scalePercent ?? 100)) / 100;
}

export function horizontalAnchorExtent(character: Pick<Character, "anchor" | "animations" | "canvasLayout">): number {
  const x = character.anchor.x;
  let extent = Math.max(x, 1 - x, character.canvasLayout?.horizontalAnchorExtent ?? 0.5);
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
  private resident = new Set<ImageAsset>();
  private residentBytes = 0;
  private loadingImages = 0;
  private bitmapRequests = 0;
  private currentImage?: ImageAsset;
  private idleImage?: ImageAsset;
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
    private loadBitmap?: (id: string) => Promise<ImageBitmap>,
  ) {
    const context = canvas.getContext("2d");
    if (!context) throw new Error("Canvas 2D를 사용할 수 없습니다.");
    this.context = context;
    this.layoutAnimations = character.animations;
    this.reservedScale = Math.max(scaleEnvelope(this.layoutAnimations), character.canvasLayout?.scaleEnvelope ?? 1);
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
    for (const [id, url] of Object.entries(
      typeof imageUrls === "string" ? { sheet: imageUrls } : imageUrls,
    )) {
      this.images.set(id, { id, url, status: "new", failures: 0, deadline: 0, revision: 0,
        bytes: typeof imageUrls === "string" ? character.sheet.width * character.sheet.height * 4 : 0 });
    }
    for (const animation of Object.values(this.plan.animations)) {
      if (!animation) continue;
      const extent = { width: 0, height: 0, duration: 0 };
      for (const frame of animation.frames) {
        extent.width = Math.max(extent.width, frame.region.width);
        extent.height = Math.max(extent.height, frame.region.height);
        extent.duration += frame.durationMs;
        const asset = this.images.get(frame.image);
        if (asset) asset.bytes = Math.max(asset.bytes,
          (frame.region.x + frame.region.width) * (frame.region.y + frame.region.height) * 4);
      }
      this.extents.set(animation, extent);
    }
    const idleFrame = this.plan.animations[character.defaultAnimation]?.frames[0];
    if (idleFrame) this.idleImage = this.images.get(idleFrame.image);
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
    this.stop();
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
    for (const asset of this.resident) this.releaseImage(asset);
    this.images.clear();
    this.currentImage = undefined;
    this.idleImage = undefined;
  }
  private releaseImage(asset: ImageAsset) {
    if (this.resident.delete(asset)) this.residentBytes -= asset.bytes;
    if (asset.status === "loading") this.loadingImages--;
    asset.revision++;
    asset.bitmap?.close();
    asset.bitmap = undefined;
    if (asset.image) {
      asset.image.onload = null;
      asset.image.onerror = null;
      asset.image.removeAttribute?.("src");
      asset.image = undefined;
    }
    asset.status = "new";
  }
  private makeRoom(bytes: number, loads: number, protectedAssets: ReadonlySet<ImageAsset>) {
    const overBytes = () => this.residentBytes + bytes > IMAGE_CACHE_BYTES;
    const fits = () => !overBytes() && this.loadingImages + loads <= IMAGE_LOAD_LIMIT;
    for (const asset of this.resident) {
      if (fits()) return true;
      if (asset !== this.idleImage && !protectedAssets.has(asset)
          && (overBytes() || asset.status === "loading")) this.releaseImage(asset);
    }
    if (!fits() && this.idleImage && this.resident.has(this.idleImage) && !protectedAssets.has(this.idleImage)
        && (overBytes() || this.idleImage.status === "loading"))
      this.releaseImage(this.idleImage);
    return fits();
  }
  private failImage(asset: ImageAsset) {
    this.releaseImage(asset);
    asset.status = "failed";
    asset.deadline = performance.now() + RETRY_DELAYS_MS[
      Math.min(asset.failures++, RETRY_DELAYS_MS.length - 1)
    ];
  }
  private prepareImage(asset: ImageAsset | undefined, now: number, protectedAssets: Set<ImageAsset>) {
    if (!asset) return;
    if (asset.status === "loading" && now >= asset.deadline)
      this.failImage(asset);
    if (this.resident.delete(asset)) {
      this.resident.add(asset);
      protectedAssets.add(asset);
      return;
    }
    if (asset.status === "failed" && now < asset.deadline)
      return;
    if (asset !== this.currentImage && asset.bytes + (this.currentImage?.bytes ?? 0) > IMAGE_CACHE_BYTES)
      return;
    if (this.loadBitmap && this.bitmapRequests >= IMAGE_LOAD_LIMIT) return;
    if (!this.makeRoom(asset.bytes, 1, protectedAssets) && asset !== this.currentImage) return;
    protectedAssets.add(asset);
    this.resident.add(asset);
    this.residentBytes += asset.bytes;
    this.loadingImages++;
    asset.status = "loading";
    asset.deadline = now + IMAGE_TIMEOUT_MS;
    const wake = () => {
      if (this.currentImage !== asset) return;
      this.stop();
      this.schedule();
    };
    const ready = (bytes: number) => {
      this.loadingImages--;
      asset.status = "ready";
      this.residentBytes += bytes - asset.bytes;
      asset.bytes = bytes;
      if (this.residentBytes > IMAGE_CACHE_BYTES) {
        const required = new Set<ImageAsset>();
        if (this.currentImage) required.add(this.currentImage);
        this.makeRoom(0, 0, required);
      }
      wake();
    };
    const failed = () => {
      this.failImage(asset);
      wake();
    };
    if (this.loadBitmap) {
      const revision = asset.revision;
      this.bitmapRequests++;
      void this.loadBitmap(asset.id).then(bitmap => {
        if (asset.revision !== revision) {
          bitmap.close();
          return;
        }
        asset.bitmap = bitmap;
        ready(bitmap.width * bitmap.height * 4);
      }, () => {
        if (asset.revision === revision) failed();
      }).finally(() => {
        this.bitmapRequests--;
        if (this.currentImage?.status === "new") {
          this.stop();
          this.schedule();
        }
      });
      return;
    }
    const image = new Image();
    // The overlay reads the composited alpha to decide which pixels can be clicked.
    if (this.onPixelsChanged) image.crossOrigin = "anonymous";
    asset.image = image;
    image.onload = () => {
      if (asset.image === image) ready(image.naturalWidth * image.naturalHeight * 4 || asset.bytes);
    };
    image.onerror = () => {
      if (asset.image === image) failed();
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
    const protectedAssets = new Set<ImageAsset>();
    this.prepareImage(asset, now, protectedAssets);
    // Decode only the current clip's nearby frames, rather than opening every
    // image in the project when the Player starts or Studio changes tabs.
    for (let offset = 1; offset <= 3 && offset < animation.frames.length; offset++) {
      const next = animation.frames[index + offset]
        ?? (animation.looping ? animation.frames[(index + offset) % animation.frames.length] : undefined);
      if (next) this.prepareImage(this.images.get(next.image), now, protectedAssets);
    }
    const nextFrame = (animation.looping && animation.frames.length > 1)
      || index < animation.frames.length - 1
      ? Math.max(1, frame.durationMs - position)
      : Infinity;
    const image = asset?.bitmap ?? asset?.image;
    if (asset?.status !== "ready" || !image) {
      if (!asset || asset.failures) this.drawError();
      const delay = Math.min(nextFrame, asset && asset.status !== "new"
        ? Math.max(1, asset.deadline - now) : Infinity);
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
          image,
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
    if (outcome === "drawn") this.drawFirstFrameImmediately = false;
    this.onFrame({
      seq: this.seq,
      epoch: this.state.epoch,
      actionInstanceId: this.state.actionInstanceId,
      outcome,
    });
  }
}
