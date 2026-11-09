import { Channel, convertFileSrc, invoke } from "@tauri-apps/api/core";
import type {
  PlayerBootstrap,
  WireState,
} from "../../../packages/contracts/generated/index";
import { CharacterRenderer } from "../../../packages/renderer/src/index";
import { ReactionAudio } from "../../../packages/renderer/src/audio";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { installCharacterDrag } from "./drag";
import { trackCharacterPixels } from "./hit-mask";
import { startSettings } from "./settings";
import "./style.css";

const app = document.querySelector<HTMLElement>("#app")!;
const settings = location.hash === "#settings";

async function startOverlay() {
  const bootstrap = await invoke<PlayerBootstrap>("get_bootstrap");
  const canvas = document.createElement("canvas");
  canvas.setAttribute("aria-label", bootstrap.product.name);
  app.append(canvas);
  const onOverlayError = (error: unknown) => { canvas.title = String(error); };
  const hitMask = trackCharacterPixels(canvas, (mask) => invoke("set_character_hit_mask", { mask }), onOverlayError);
  const sound = new ReactionAudio(
    (resource) => {
      const path = bootstrap.soundPaths[resource];
      return path ? convertFileSrc(path) : undefined;
    },
    (error) => { canvas.title = `사운드를 재생하지 못했습니다: ${String(error)}`; },
    (resource) => invoke<ArrayBuffer>("read_reaction_audio", { resource }),
  );
  let latest: WireState | undefined;
  let rendered: WireState | undefined;
  // The native window starts hidden until its first frame is ready. Allow that
  // first draw even if WebView reports document.hidden during bootstrap.
  let firstFramePending = true;
  let audioWarmupStarted = false;
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
      if (ack.outcome === "drawn" && rendered?.seq === ack.seq &&
          latest?.state.epoch === ack.epoch && latest.state.actionInstanceId === ack.actionInstanceId) {
        sound.update(rendered.state, hostVisible && !document.hidden);
      }
      void invoke("ack_character_frame", { ack })
        .then(() => {
          if (!audioWarmupStarted && ack.outcome === "drawn") {
            audioWarmupStarted = true;
            void sound.preload(Object.keys(bootstrap.soundPaths));
          }
          firstFramePending = false;
          renderer.setVisible(hostVisible && !document.hidden);
        })
        .catch((error) => {
          canvas.title = String(error);
        });
    },
    bootstrap.renderPlan,
    true,
    hitMask.update,
  );
  let width = bootstrap.width,
    height = bootstrap.height;
  const resizeCanvas = () => renderer.resize(width, height);
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
  const channel = new Channel<WireState>();
  channel.onmessage = (message) => {
    const distance = lastSeq === null ? 1 : (message.seq - lastSeq) >>> 0;
    if (distance === 0 || distance > 0x7fffffff) return;
    lastSeq = message.seq;
    if (!message.visible || latest?.state.epoch !== message.state.epoch ||
        latest.state.actionInstanceId !== message.state.actionInstanceId) sound.stop();
    latest = message;
    app.classList.toggle("editing", message.editing);
    hostVisible = message.visible;
    renderer.setVisible(hostVisible && (firstFramePending || !document.hidden));
    width = message.width;
    height = message.height;
    const { x, y } = message.canvasOffset;
    canvas.style.transform = `translate(${x}px, ${y}px)`;
    hitMask.place(x / width, y / height);
    resizeCanvas();
    const draw = () => {
      if (latest !== message) return;
      rendered = message;
      renderer.update(message.state, message.seq);
    };
    const cue = message.state.sound;
    const loading = message.visible && cue?.volume ? sound.waitForCue(cue.resource) : undefined;
    if (loading) void loading.then(draw);
    else draw();
  };
  await invoke("subscribe_character_state", { channel });
  installCharacterDrag(canvas, invoke, onOverlayError, hitMask.contains);
  canvas.oncontextmenu = (event) => {
    event.preventDefault();
    if (hitMask.contains(event)) {
      void invoke("show_buddy_menu", { position: { x: event.clientX, y: event.clientY } }).catch(onOverlayError);
    }
  };
  document.addEventListener("visibilitychange", () => {
    renderer.setVisible(hostVisible && (firstFramePending || !document.hidden));
    if (document.hidden) sound.stop();
    else sound.activate();
  });
}

(settings ? startSettings({
  invoke,
  close: () => getCurrentWindow().destroy(),
  onCloseRequested: (callback) => getCurrentWindow().onCloseRequested((event) => { event.preventDefault(); callback(); }),
}) : startOverlay()).catch((error) => {
  app.textContent = `${settings ? "설정을 불러오지 못했습니다." : "캐릭터를 준비하지 못했습니다."} ${String(error)}`;
});
