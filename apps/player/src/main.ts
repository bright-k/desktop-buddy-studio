import { Channel, convertFileSrc, invoke } from "@tauri-apps/api/core";
import type {
  PlayerBootstrap,
  WireState,
} from "../../../packages/contracts/generated/index";
import { CharacterRenderer } from "../../../packages/renderer/src/index";
import { ReactionAudio } from "../../../packages/renderer/src/audio";
import { installCharacterDrag } from "./drag";
import { trackCharacterPixels } from "./hit-mask";
import { fitOverlayCanvas } from "./overlay-layout";
import "./style.css";

const app = document.querySelector<HTMLElement>("#app")!;
const mode = location.hash;

async function startOverlay() {
  const generation = Number(new URLSearchParams(location.search).get("overlay-recovery") ?? 0);
  const overlayInvoke = <T = unknown>(command: string, args?: Record<string, unknown>) =>
    invoke<T>(command, { ...args, generation });
  const readBytes = async (command: string, args: Record<string, unknown>) => {
    const response = await overlayInvoke<ArrayBuffer | number[]>(command, args);
    // Tauri's postMessage fallback serializes binary responses as an array.
    return Array.isArray(response) ? Uint8Array.from(response).buffer : response;
  };
  const bootstrap = await overlayInvoke<PlayerBootstrap>("get_bootstrap");
  const canvas = document.createElement("canvas");
  canvas.setAttribute("aria-label", bootstrap.product.name);
  app.append(canvas);
  const onOverlayError = (error: unknown) => { canvas.title = String(error); };
  const hitMask = trackCharacterPixels(canvas, (mask) => overlayInvoke("set_character_hit_mask", { mask }), onOverlayError);
  const sound = new ReactionAudio(
    (resource) => {
      const path = bootstrap.soundPaths[resource];
      return path ? convertFileSrc(path) : undefined;
    },
    (error) => { canvas.title = `사운드를 재생하지 못했습니다: ${String(error)}`; },
    (resource, sampleRate, maxDecodedBytes) =>
      readBytes("read_reaction_audio", { resource, sampleRate, maxDecodedBytes }),
  );
  sound.setEnabled(bootstrap.soundEnabled);
  let latest: WireState | undefined;
  let rendered: WireState | undefined;
  let drawn: WireState | undefined;
  let restoredAction: string | undefined;
  const actionKey = ({ epoch, actionInstanceId }: WireState["state"]) => `${epoch}:${actionInstanceId}`;
  const playDrawnSound = () => {
    if (!document.hidden && latest?.visible && drawn?.state.epoch === latest.state.epoch &&
        drawn.state.actionInstanceId === latest.state.actionInstanceId &&
        actionKey(drawn.state) !== restoredAction) sound.update(drawn.state);
  };
  // The native window starts hidden until its first frame is ready. Allow that
  // first draw even if WebView reports document.hidden during bootstrap.
  let firstFramePending = true;
  let audioWarmupStarted = false;
  const warmSound = () => {
    if (!audioWarmupStarted && drawn && latest?.soundEnabled) {
      audioWarmupStarted = true;
      void sound.preload(Object.keys(bootstrap.soundPaths));
    }
  };
  const renderer = new CharacterRenderer(
    canvas,
    bootstrap.character,
    Object.fromEntries(
      Object.entries(bootstrap.imagePaths).map(([id, path]) => [
        id,
        convertFileSrc(path!),
      ]),
    ),
    (ack) => {
      if (ack.outcome === "drawn" && rendered?.seq === ack.seq) {
        drawn = rendered;
        playDrawnSound();
      }
      void overlayInvoke("ack_character_frame", { ack })
        .then(() => {
          warmSound();
          if (ack.outcome === "drawn" || generation === 0) firstFramePending = false;
          renderer.setVisible(hostVisible && (firstFramePending || !document.hidden));
          playDrawnSound();
        })
        .catch((error) => {
          canvas.title = String(error);
        });
    },
    bootstrap.renderPlan,
    true,
    hitMask.update,
    // Explicitly close evicted bitmaps. WebKit can retain decoded <img>
    // resources after the last element referencing them has been released.
    typeof createImageBitmap === "function" ? async (frame) => {
      try {
        const bytes = await readBytes("read_render_frame", { frame });
        return await createImageBitmap(new Blob([bytes], { type: "image/png" }));
      } catch (error) {
        onOverlayError(error);
        throw error;
      }
    } : undefined,
  );
  let width = bootstrap.width,
    height = bootstrap.height;
  let offsetX = 0, offsetY = 0, canvasScale = 1;
  const resizeCanvas = () => {
    const layout = fitOverlayCanvas(width, height, window.innerWidth, window.innerHeight);
    canvasScale = layout.scale;
    canvas.style.transform = `translate(${offsetX * canvasScale}px, ${offsetY * canvasScale}px)`;
    hitMask.place(offsetX / width * layout.fractionX, offsetY / height * layout.fractionY,
      layout.fractionX, layout.fractionY);
    renderer.resize(layout.width, layout.height);
  };
  const watchPixelRatio = () => {
    const query = matchMedia(
      `(resolution: ${window.devicePixelRatio || 1}dppx)`,
    );
    query.addEventListener(
      "change",
      () => {
        resizeCanvas();
        watchPixelRatio();
      },
      { once: true },
    );
  };
  resizeCanvas();
  watchPixelRatio();
  window.addEventListener("resize", resizeCanvas);
  let lastSeq: number | null = null;
  let hostVisible = true;
  const cancelGesture = installCharacterDrag(canvas, overlayInvoke, onOverlayError, hitMask.contains, () => canvasScale);
  const channel = new Channel<WireState>();
  channel.onmessage = (message) => {
    const distance = lastSeq === null ? 1 : (message.seq - lastSeq) >>> 0;
    if (distance === 0 || distance > 0x7fffffff) return;
    if (lastSeq === null && generation !== 0) restoredAction = actionKey(message.state);
    lastSeq = message.seq;
    if (!message.visible) cancelGesture();
    if (!message.visible || latest?.state.epoch !== message.state.epoch ||
        latest.state.actionInstanceId !== message.state.actionInstanceId) sound.stop();
    latest = message;
    sound.setEnabled(message.soundEnabled);
    if (!message.soundEnabled) audioWarmupStarted = false;
    app.classList.toggle("editing", message.editing);
    hostVisible = message.visible;
    renderer.setVisible(hostVisible && (firstFramePending || !document.hidden));
    const { x, y } = message.canvasOffset;
    if (width !== message.width || height !== message.height || offsetX !== x || offsetY !== y) {
      width = message.width;
      height = message.height;
      offsetX = x;
      offsetY = y;
      resizeCanvas();
    }
    const cue = message.state.sound;
    if (message.visible && cue?.volume && actionKey(message.state) !== restoredAction)
      void sound.prepare(cue.resource);
    rendered = message;
    renderer.update(message.state, message.seq);
    if (!firstFramePending) warmSound();
  };
  document.addEventListener("visibilitychange", () => {
    renderer.setVisible(hostVisible && (firstFramePending || !document.hidden));
    if (document.hidden) {
      cancelGesture();
      sound.stop();
    }
    else {
      void sound.activate();
      playDrawnSound();
    }
  });
  window.addEventListener("pagehide", () => {
    renderer.dispose();
    sound.dispose();
    canvas.width = canvas.height = 0;
  }, { once: true });
  await overlayInvoke("subscribe_character_state", { channel });
  canvas.oncontextmenu = (event) => {
    event.preventDefault();
    if (hitMask.contains(event)) {
      void invoke("show_buddy_menu", { position: { x: event.clientX, y: event.clientY } }).catch(onOverlayError);
    }
  };
}

async function startPlayer() {
  if (mode === "#settings") {
    const [{ startSettings }, { getCurrentWindow }] = await Promise.all([
      import("./settings"), import("@tauri-apps/api/window"),
    ]);
    const window = getCurrentWindow();
    return startSettings({
      invoke,
      close: () => window.destroy(),
      onCloseRequested: (callback) => window.onCloseRequested((event) => { event.preventDefault(); callback(); }),
    });
  }
  if (mode === "#timer" || mode === "#timer-badge" || mode === "#timer-quick") {
    const { startTimer, startTimerBadge, startTimerQuick } = await import("./timer");
    return { "#timer": startTimer, "#timer-badge": startTimerBadge, "#timer-quick": startTimerQuick }[mode](app);
  }
  return startOverlay();
}

startPlayer().catch((error) => {
  app.textContent = `${mode === "#settings" ? "설정을 불러오지 못했습니다." : mode.startsWith("#timer") ? "타이머를 열지 못했습니다." : "캐릭터를 준비하지 못했습니다."} ${String(error)}`;
});
