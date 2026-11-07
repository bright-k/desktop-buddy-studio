import { Channel, convertFileSrc, invoke } from "@tauri-apps/api/core";
import type {
  InputSourceStatus,
  PlayerBootstrap,
  RuntimeStatus,
  WireState,
} from "../../../packages/contracts/generated/index";
import { CharacterRenderer } from "../../../packages/renderer/src/index";
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
  let pointer: { x: number; y: number; id: number } | null = null;
  let dragSession: string | null = null;
  let operation = Promise.resolve();
  const enqueue = (work: () => Promise<unknown>) => {
    operation = operation
      .then(async () => {
        await work();
      })
      .catch((error) => {
        canvas.title = String(error);
      });
  };
  canvas.title = "드래그해서 위치 이동 · 크기는 트레이 설정에서 조절";
  canvas.onpointerdown = (event) => {
    if (event.button !== 0 || pointer) return;
    pointer = { x: event.screenX, y: event.screenY, id: event.pointerId };
    canvas.setPointerCapture(event.pointerId);
    enqueue(async () => {
      dragSession = await invoke<string>("begin_character_drag");
    });
  };
  canvas.onpointermove = (event) => {
    if (!pointer || event.pointerId !== pointer.id) return;
    const dx = event.screenX - pointer.x,
      dy = event.screenY - pointer.y;
    pointer = { x: event.screenX, y: event.screenY, id: event.pointerId };
    if (dx || dy)
      enqueue(async () => {
        if (dragSession)
          await invoke("update_character_edit", {
            sessionId: dragSession,
            dx,
            dy,
          });
      });
  };
  const finishDrag = () => {
    if (!pointer) return;
    pointer = null;
    enqueue(async () => {
      if (dragSession) {
        const sessionId = dragSession;
        dragSession = null;
        await invoke("finish_character_edit", { sessionId });
      }
    });
  };
  canvas.onpointerup = finishDrag;
  canvas.onpointercancel = finishDrag;
  canvas.onlostpointercapture = finishDrag;
  document.addEventListener("visibilitychange", () =>
    renderer.setVisible(hostVisible && (firstFramePending || !document.hidden)),
  );
}

async function startSettings() {
  document.body.classList.add("settings");
  app.innerHTML =
    '<header><span class="eyebrow">DESKTOP BUDDY</span><h1>나의 데스크톱 친구</h1><p>다른 앱을 사용하는 동안에도 키보드와 마우스에 반응합니다.</p></header><section><h2>입력과 표시</h2><label><input id="keyboard" type="checkbox"> 키보드 · 입력 속도 · 키워드</label><label><input id="mouse" type="checkbox"> 마우스 반응</label><label><input id="movement" type="checkbox"> 키 유지 이동</label><label><input id="top" type="checkbox"> 항상 위에 표시</label><label class="scale">캐릭터 크기 <input id="scale" type="range" min="25" max="300" step="5"><output id="scale-value"></output></label></section><section><h2>기본 위치</h2><label>디스플레이 <select id="monitor"></select></label><div class="coordinates"><label>X <input id="position-x" type="number" min="0" max="1000000" step="1"></label><label>Y <input id="position-y" type="number" min="0" max="1000000" step="1"></label></div><p class="hint">선택한 디스플레이 작업 영역의 왼쪽 위가 (0, 0)입니다. 화면의 버디를 바로 드래그해 옮길 수 있습니다. 크기는 위 슬라이더에서 조절합니다.</p></section><section><h2>입력 감지 상태</h2><p id="status-keyboard" role="status"></p><p id="status-mouse" role="status"></p><button id="permission">입력 권한 요청 · 다시 확인</button><p class="hint">한글 키워드는 두벌식 물리 키 순서로 반응합니다. 현재 한·영 모드와 관계없이 같은 키 순서면 동작합니다.</p></section><footer><button id="reset-position">위치 초기화</button><button id="reset-all">전체 설정 초기화</button><button id="save" class="primary">설정 저장</button></footer><p id="message" role="status"></p>';
  document
    .querySelector("footer")!
    .insertAdjacentHTML(
      "beforebegin",
      '<section><h2>반응 타이밍</h2><div class="coordinates"><label>입력 중단 (ms)<input id="idle-timeout" type="number" min="50" max="5000" step="10"></label><label>속도 안정 (ms)<input id="rate-stability" type="number" min="0" max="2000" step="10"></label></div><details><summary>속도 측정 설정</summary><div class="coordinates"><label>측정 구간 (ms)<input id="rate-window" type="number" min="100" max="10000" step="50"></label><label>평가 간격 (ms)<input id="rate-evaluation" type="number" min="10" max="250" step="10"></label></div></details><p class="hint">입력 중단은 마지막 일반 키 입력부터 대기로 돌아가기까지의 시간입니다. 일회성 동작의 길이는 Studio 이벤트에서 설정합니다.</p></section>',
    );
  const input = (id: string) => document.getElementById(id) as HTMLInputElement;
  const message = document.getElementById("message")!;
  document
    .querySelector("footer")!
    .insertAdjacentHTML(
      "beforebegin",
      '<section><details><summary>반응 성능 진단</summary><p class="hint">계측을 초기화한 뒤 설정 창을 닫고 시험하세요. 30 FPS 일회성 반응과 다른 FPS를 구분합니다. 이 수치는 화면 발광 지연이나 성능 통과 판정이 아닙니다.</p><pre id="performance"></pre><button id="perf-refresh">집계 새로고침</button><button id="perf-reset">계측 초기화</button><button id="perf-export">진단 JSON 저장</button></details></section>',
    );
  const showPerformance = (status: RuntimeStatus) => {
    document.getElementById("performance")!.textContent = JSON.stringify(
      status.performance,
      null,
      2,
    );
  };
  async function refresh() {
    const status = await invoke<RuntimeStatus>("get_runtime_status");
    input("keyboard").checked = status.keyboard;
    input("mouse").checked = status.mouse;
    input("movement").checked = status.movementEnabled;
    input("top").checked = status.alwaysOnTop;
    input("scale").value = String(status.scale * 100);
    document.getElementById("scale-value")!.textContent =
      `${Math.round(status.scale * 100)}%`;
    const select = document.getElementById("monitor") as HTMLSelectElement;
    select.replaceChildren(
      ...status.monitors.map((monitor) => {
        const option = document.createElement("option");
        option.value = monitor.id;
        option.textContent = `${monitor.name}${monitor.primary ? " · 주 디스플레이" : ""} (${Math.round(monitor.workArea.width)} × ${Math.round(monitor.workArea.height)})`;
        return option;
      }),
    );
    select.value = status.basePlacement.monitorHint;
    input("position-x").value = String(
      Math.round(status.basePlacement.position.x),
    );
    input("position-y").value = String(
      Math.round(status.basePlacement.position.y),
    );
    input("idle-timeout").value = String(status.timing.idleTimeoutMs);
    input("rate-stability").value = String(status.timing.rateStabilityMs);
    input("rate-window").value = String(status.timing.rateWindowMs);
    input("rate-evaluation").value = String(status.timing.rateEvaluationMs);
    showInputStatus(status);
    showPerformance(status);
    message.textContent = status.notice ?? "";
  }
  function showInputStatus(status: RuntimeStatus) {
    const describe = (source: InputSourceStatus) => {
      const permission = {
        notRequired: "별도 권한 필요 없음",
        needsRequest: "입력 권한 필요",
        granted: "권한 허용",
        denied: "권한 거부",
      }[source.permission];
      const capture = {
        stopped: "감지 중지",
        running: "감지 중",
        suspended: "감지 보류",
        failed: "감지 실패",
      }[source.capture];
      return `${permission} · ${capture}${source.reason ? ` — ${source.reason}` : ""}`;
    };
    document.getElementById("status-keyboard")!.textContent =
      `키보드: ${describe(status.input.keyboard)}`;
    document.getElementById("status-mouse")!.textContent =
      `마우스: ${describe(status.input.mouse)}`;
  }
  const action = async (work: () => Promise<unknown>) => {
    try {
      await work();
    } catch (error) {
      message.textContent = String(error);
    }
  };
  input("scale").oninput = () => {
    document.getElementById("scale-value")!.textContent =
      `${input("scale").value}%`;
  };
  document.getElementById("permission")!.onclick = () =>
    void action(async () => {
      await invoke("request_input_access");
      showInputStatus(await invoke<RuntimeStatus>("get_runtime_status"));
    });
  document.getElementById("save")!.onclick = async () => {
    try {
      await invoke("apply_user_preferences", {
        update: {
          timing: {
            idleTimeoutMs: Number(input("idle-timeout").value),
            rateStabilityMs: Number(input("rate-stability").value),
            rateWindowMs: Number(input("rate-window").value),
            rateEvaluationMs: Number(input("rate-evaluation").value),
          },
          keyboard: input("keyboard").checked,
          mouse: input("mouse").checked,
          movementEnabled: input("movement").checked,
          alwaysOnTop: input("top").checked,
          scale: Number(input("scale").value) / 100,
          basePlacement: {
            monitorHint: (
              document.getElementById("monitor") as HTMLSelectElement
            ).value,
            position: {
              x: Number(input("position-x").value),
              y: Number(input("position-y").value),
            },
          },
        },
      });
      message.textContent =
        "저장했습니다. 창을 닫으면 입력 반응이 다시 시작됩니다.";
    } catch (error) {
      message.textContent = String(error);
    }
  };
  document.getElementById("reset-position")!.onclick = () =>
    void action(() =>
      invoke("reset_user_preferences", { all: false }).then(refresh),
    );
  document.getElementById("reset-all")!.onclick = () =>
    void action(() =>
      invoke("reset_user_preferences", { all: true }).then(refresh),
    );
  document.getElementById("perf-refresh")!.onclick = () =>
    void action(async () =>
      showPerformance(await invoke<RuntimeStatus>("get_runtime_status")),
    );
  document.getElementById("perf-reset")!.onclick = () =>
    void action(async () => {
      await invoke("reset_performance_session");
      showPerformance(await invoke<RuntimeStatus>("get_runtime_status"));
      message.textContent =
        "계측을 초기화했습니다. 설정 창을 닫고 시험하세요. 최초 프레임 시간은 유지됩니다.";
    });
  document.getElementById("perf-export")!.onclick = () =>
    void action(async () => {
      const path = await invoke<string>("export_runtime_diagnostics");
      message.textContent = `진단을 저장했습니다: ${path}`;
    });
  await refresh();
}
(settings ? startSettings() : startOverlay()).catch((error) => {
  app.textContent = `캐릭터를 준비하지 못했습니다: ${String(error)}`;
});
