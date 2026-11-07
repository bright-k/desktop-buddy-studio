import { Channel, convertFileSrc, invoke } from "@tauri-apps/api/core";
import type {
  PlayerBootstrap,
  WireState,
} from "../../../packages/contracts/generated/index";
import { CharacterRenderer } from "../../../packages/renderer/src/index";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { installCharacterDrag } from "./drag";
import { startSettings } from "./settings";
import "./style.css";

const app = document.querySelector<HTMLElement>("#app")!;
const settings = location.hash === "#settings";

async function startOverlay() {
  const bootstrap = await invoke<PlayerBootstrap>("get_bootstrap");
  const canvas = document.createElement("canvas");
  canvas.setAttribute("aria-label", bootstrap.product.name);
  app.append(canvas);
  // The native window starts hidden until its first frame is ready. Allow that
  // first draw even if WebView reports document.hidden during bootstrap.
  let firstFramePending = true;
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
      void invoke("ack_character_frame", { ack })
        .then(() => {
          firstFramePending = false;
          renderer.setVisible(hostVisible && !document.hidden);
        })
        .catch((error) => {
          canvas.title = String(error);
        });
    },
    bootstrap.renderPlan,
    true,
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
    app.classList.toggle("editing", message.editing);
    hostVisible = message.visible;
    renderer.setVisible(hostVisible && (firstFramePending || !document.hidden));
    width = message.width;
    height = message.height;
    resizeCanvas();
    renderer.update(message.state, message.seq);
  };
  await invoke("subscribe_character_state", { channel });
  installCharacterDrag(canvas, invoke, (error) => { canvas.title = String(error); });
  document.addEventListener("visibilitychange", () =>
    renderer.setVisible(hostVisible && (firstFramePending || !document.hidden)),
  );
}

(settings ? startSettings({
  invoke,
  close: () => getCurrentWindow().destroy(),
  onCloseRequested: (callback) => getCurrentWindow().onCloseRequested((event) => { event.preventDefault(); callback(); }),
}) : startOverlay()).catch((error) => {
  app.textContent = `캐릭터를 준비하지 못했습니다: ${String(error)}`;
});
