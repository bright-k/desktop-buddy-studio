import type { InputSourceStatus, RuntimeStatus } from "../../../packages/contracts/generated/index";
import { mergeSettings, settingsUpdate, settingsValues, type SettingsValues } from "./settings-model";

type SettingsHost = {
  invoke<T = unknown>(command: string, args?: Record<string, unknown>): Promise<T>;
  close(): Promise<void>;
  onCloseRequested?(callback: () => void): Promise<() => void>;
};
export async function startSettings(host: SettingsHost) {
  document.body.classList.add("settings");
  const app = document.querySelector<HTMLElement>("#app")!;
  app.innerHTML = `<header><span class="eyebrow">DESKTOP BUDDY · 설정</span><h1 id="product-name">버디 설정</h1><p>표시와 입력 설정은 저장하면 적용됩니다. 자동 실행은 변경 즉시 적용됩니다.</p></header>
    <div id="runtime-banner" class="runtime-banner" role="status">설정을 불러오고 있습니다.</div><p id="notice" class="notice" hidden></p>
    <form id="preferences"><fieldset id="settings-fields" disabled>
    <section><h2>입력과 표시</h2><label><input id="keyboard" type="checkbox"> 키보드 · 입력 속도 · 키워드</label><label><input id="mouse" type="checkbox"> 마우스 반응</label><label><input id="movement" type="checkbox"> 키를 누른 채 이동</label><label><input id="cursor-follow" type="checkbox"> 마우스 따라오기</label><label><input id="top" type="checkbox"> 항상 위에 표시</label><label class="scale">캐릭터 크기 <input id="scale" type="range" min="25" max="300" step="1"><output id="scale-value"></output></label><p class="hint">화면 작업 영역보다 커지면 표시 크기를 제한합니다.</p></section>
    <section><h2>위치</h2><label class="monitor-field">디스플레이 <select id="monitor" required></select></label><div class="coordinates"><label>X <input id="position-x" type="number" required min="-1000000" max="1000000" step="1"></label><label>Y <input id="position-y" type="number" required min="-1000000" max="1000000" step="1"></label></div><p class="hint">선택한 화면의 왼쪽 위가 (0, 0)입니다. 투명 여백에 따라 좌표가 음수일 수 있습니다. 버디를 드래그하면 위치가 바로 저장됩니다.</p></section>
    <section><h2>반응 타이밍</h2><div class="coordinates"><label>입력 중단 (ms)<input id="idle-timeout" type="number" required min="50" max="5000" step="1"></label><label>속도 구간 안정 시간 (ms)<input id="rate-stability" type="number" required min="0" max="2000" step="1"></label></div><p class="hint">입력 중단은 마지막 일반 키 입력 뒤 입력 속도 애니메이션을 끝내기까지의 시간입니다. 일회성 애니메이션 길이는 Studio 이벤트에서 조절합니다.</p><details><summary>속도 측정 설정</summary><div class="coordinates"><label>속도 측정 구간 (ms)<input id="rate-window" type="number" required min="100" max="10000" step="1"></label><label>속도 평가 간격 (ms)<input id="rate-evaluation" type="number" required min="10" max="250" step="1"></label></div></details></section>
    </fieldset></form>
    <section><h2>자동 실행</h2><label><input id="launch-at-login" type="checkbox" disabled> 로그인할 때 버디 실행</label><p id="launch-at-login-status" class="hint" role="status">자동 실행 상태를 불러오는 중입니다.</p></section>
    <section><h2>입력 감지와 권한</h2><p id="status-keyboard"></p><p id="status-mouse"></p><div class="settings-button-row"><button id="input-settings" type="button" hidden>입력 모니터링 설정 열기</button><button id="permission" type="button">입력 상태 다시 확인</button></div><p class="hint">설정 창을 닫으면 입력 감지를 재개합니다. 한글 키워드는 현재 한·영 모드와 관계없이 두벌식 물리 키 순서로 판정합니다.</p></section>
    <section><details><summary>초기화 · 성능 진단</summary><p class="hint">초기화 전에 변경사항을 확인합니다. 전체 초기화는 키보드·마우스 감지, 마우스 따라오기와 자동 실행도 끕니다.</p><div class="settings-button-row"><button id="reset-position">위치 초기화</button><button id="reset-all" class="danger">전체 설정 초기화</button></div><h2 class="diagnostic-heading">반응 성능 진단</h2><p class="hint">진단 기록 지우기 후 설정 창을 닫고 시험하세요. 화면 발광 지연이나 성능 통과 판정 수치는 아닙니다.</p><pre id="performance"></pre><div class="settings-button-row"><button id="perf-refresh">진단 결과 보기</button><button id="perf-reset">진단 기록 지우기</button><button id="perf-export">진단 JSON 저장</button></div></details></section>
    <footer class="settings-actions"><span id="save-state" role="status">불러오는 중</span><div class="settings-button-row"><button id="close-settings">닫기</button><button id="save">저장</button><button id="save-close" class="primary">저장하고 닫기</button></div><p id="message" role="status" aria-live="polite"></p></footer>
    <dialog id="close-dialog"><h2>저장하지 않은 변경이 있습니다.</h2><p>설정을 적용하고 닫거나, 변경을 버릴 수 있습니다.</p><p id="dialog-message" class="error-text" role="alert" hidden></p><div class="settings-button-row"><button id="keep-editing">계속 편집</button><button id="discard-close" class="danger">버리고 닫기</button><button id="confirm-save" class="primary">저장하고 닫기</button></div></dialog>`;
  const input = (id: string) => document.getElementById(id) as HTMLInputElement;
  const element = (id: string) => document.getElementById(id)!;
  const form = element("preferences") as HTMLFormElement;
  const dialog = element("close-dialog") as HTMLDialogElement;
  let baseline: SettingsValues = {}, status: RuntimeStatus | null = null, busy = false, disposed = false, request = 0;
  let unlisten: (() => void) | undefined, timer = 0;
  const values = (): SettingsValues => Object.fromEntries(Object.keys(baseline).map((key) => [key, typeof baseline[key] === "boolean" ? input(key).checked : input(key).value]));
  const dirty = () => Object.entries(values()).some(([key, value]) => value !== baseline[key]);
  const update = () => {
    const changed = dirty();
    element("save-state").textContent = busy ? "처리 중…" : !status ? "불러오는 중" : changed ? "저장하지 않은 변경" : "모든 변경 저장됨";
    input("settings-fields").disabled = busy || !status;
    const launchAtLogin = status?.launchAtLogin;
    const launchToggle = input("launch-at-login");
    launchToggle.disabled = busy || !!status?.dragging || !launchAtLogin || launchAtLogin === "unavailable";
    launchToggle.checked = launchAtLogin === "on" || launchAtLogin === "needsApproval" || launchAtLogin === "pathChanged";
    element("launch-at-login-status").textContent = launchAtLogin ? {
      off: "켜면 다음 로그인부터 자동으로 실행합니다. 변경은 바로 적용됩니다.",
      on: status?.input.keyboard.permission === "notRequired"
        ? "Windows 시작 앱에 등록됨. Windows 설정에서 끈 경우 시작 앱 목록에서 다시 켜세요."
        : "다음 로그인부터 자동으로 실행합니다.",
      needsApproval: "시스템 설정의 로그인 항목에서 이 앱을 허용하세요.",
      pathChanged: "이전 설치 위치가 등록되어 있습니다. 끈 뒤 다시 켜면 현재 위치로 등록합니다.",
      unavailable: "현재 실행 환경에서는 자동 실행을 설정할 수 없습니다.",
    }[launchAtLogin] : "자동 실행 상태를 불러오는 중입니다.";
    for (const id of ["save", "save-close"]) input(id).disabled = busy || !status || !!status.dragging || !changed;
    for (const id of ["reset-position", "reset-all", "input-settings", "permission", "perf-refresh", "perf-reset", "perf-export"]) input(id).disabled = busy || !status || !!status.dragging;
    input("close-settings").disabled = busy;
    element("scale-value").textContent = `${input("scale").value}%`;
  };
  const say = (text: string, error = false) => {
    element("message").textContent = text; element("message").classList.toggle("error-text", error);
    const warning = element("dialog-message"); warning.textContent = error ? text : ""; warning.hidden = !error;
  };
  const put = (data: SettingsValues) => {
    for (const [key, value] of Object.entries(data)) {
      if (typeof value === "boolean") input(key).checked = value;
      else if (input(key).value !== value) input(key).value = value;
    }
  };
  const describe = (enabled: boolean, source: InputSourceStatus) => {
    if (!enabled) return "꺼짐 · 입력 감지를 켜고 저장하세요.";
    const permission = { notRequired: "별도 권한 필요 없음", needsRequest: "입력 권한 필요", granted: "권한 허용", denied: "권한 거부" }[source.permission];
    return source.permission === "needsRequest" || source.permission === "denied" ? `${permission} · 권한 허용 후 감지할 수 있습니다.` : `${permission} · 설정 중 감지 일시 정지`;
  };
  async function refresh(force = false, replace: boolean | "position" = force) {
    if (disposed || (busy && !force)) return;
    const currentRequest = ++request;
    const next = await host.invoke<RuntimeStatus>("get_runtime_status");
    if (disposed || currentRequest !== request || (busy && !force)) return;
    const incoming = settingsValues(next);
    const merged = replace === true || !status ? incoming : mergeSettings(values(), baseline, incoming);
    if (replace === "position") {
      for (const key of ["monitor", "position-x", "position-y"]) merged[key] = incoming[key];
    }
    const monitor = input("monitor") as unknown as HTMLSelectElement;
    const options = next.monitors.map((m) => ({ value: m.id, text: `${m.name}${m.primary ? " · 주 화면" : ""} (${Math.round(m.workArea.width)} × ${Math.round(m.workArea.height)})` }));
    if (JSON.stringify([...monitor.options].map((o) => ({ value: o.value, text: o.text }))) !== JSON.stringify(options))
      monitor.replaceChildren(...options.map(({ value, text }) => new Option(text, value)));
    baseline = incoming; status = next; put(merged);
    element("product-name").textContent = next.productName;
    element("runtime-banner").textContent = next.dragging ? "버디 이동 중 · 놓으면 위치가 저장됩니다." : next.paused ? "입력 일시 정지 상태입니다. 버디 메뉴에서 재개할 수 있습니다." : !next.visible ? "버디가 숨겨져 있습니다. 버디 메뉴에서 표시할 수 있습니다." : "설정 창이 열려 있는 동안 입력 감지를 멈춥니다.";
    element("notice").textContent = next.notice ?? ""; element("notice").hidden = !next.notice;
    element("status-keyboard").textContent = `키보드: ${describe(next.keyboard, next.input.keyboard)}`;
    element("status-mouse").textContent = `마우스: ${describe(next.mouse, next.input.mouse)}`;
    element("input-settings").hidden = next.input.keyboard.permission === "notRequired";
    update();
  }
  async function close() {
    if (disposed) return;
    await host.close(); disposed = true; clearInterval(timer); unlisten?.();
    window.removeEventListener("keydown", keydown); window.removeEventListener("focus", focus);
  }
  async function save(closeAfter = false) {
    if (busy || !status) return;
    if (status.dragging) { say("버디를 놓은 뒤 설정을 저장해주세요.", true); return; }
    try {
      const details = form.querySelector(":invalid:is(input,select)")?.closest("details");
      if (details) details.open = true;
      if (!form.reportValidity()) { say("표시된 입력 항목을 확인해주세요.", true); return; }
      busy = true; request++; update();
      await refresh(true, false);
      if (status.dragging) { say("버디를 놓은 뒤 설정을 저장해주세요.", true); return; }
      const updateValues = settingsUpdate(values(), baseline);
      await host.invoke("apply_user_preferences", { update: updateValues });
      await refresh(true);
      say("설정을 저장했습니다.");
      if (dialog.open) dialog.close();
      if (closeAfter) await close();
    } catch (error) { say(String(error instanceof Error ? error.message : error), true); }
    finally { busy = false; if (!disposed) update(); }
  }
  const requestClose = () => {
    if (busy) return;
    if (dirty()) { if (!dialog.open) { say(""); dialog.showModal(); } }
    else void close().catch((e) => say(String(e), true));
  };
  async function action(work: () => Promise<unknown>) {
    if (busy) return;
    busy = true; request++; update();
    try { await work(); } catch (e) { say(String(e instanceof Error ? e.message : e), true); }
    finally { busy = false; update(); }
  }
  form.addEventListener("input", () => { say(""); update(); });
  form.onsubmit = (event) => { event.preventDefault(); void save(); };
  element("save").onclick = () => void save();
  element("save-close").onclick = element("confirm-save").onclick = () => void save(true);
  input("launch-at-login").onchange = (event) => {
    const enabled = (event.currentTarget as HTMLInputElement).checked;
    void action(async () => {
      say("");
      try {
        await host.invoke("set_launch_at_login", { enabled });
      } catch (error) {
        await refresh(true, false).catch(() => {});
        throw error;
      }
      await refresh(true, false);
    });
  };
  element("close-settings").onclick = requestClose;
  element("keep-editing").onclick = () => dialog.close();
  element("discard-close").onclick = () => void close().catch((e) => say(String(e), true));
  for (const [id, all] of [["reset-position", false], ["reset-all", true]] as const)
    element(id).onclick = () => {
      if (!confirm(all ? "전체 설정을 초기화할까요? 키보드·마우스 감지, 마우스 따라오기와 자동 실행을 끄고 저장하지 않은 변경을 버립니다." : "버디를 기본 위치로 옮길까요? 편집 중인 좌표도 기본 위치로 바뀝니다.")) return;
      void action(async () => {
        try {
          await host.invoke("reset_user_preferences", { all });
        } catch (error) {
          await refresh(true, false).catch(() => {});
          throw error;
        }
        await refresh(true, all ? true : "position");
        say(all ? "전체 설정을 초기화했습니다." : "위치를 초기화했습니다.");
      });
    };
  element("input-settings").onclick = () => void action(async () => {
    await host.invoke("request_input_access");
    await refresh(true, false);
    say("시스템 설정에서 버디를 허용하세요. 목록에 없으면 + 버튼으로 앱을 추가할 수 있습니다.");
  });
  element("permission").onclick = () => void action(async () => { await refresh(true, false); say("입력 권한 상태를 갱신했습니다."); });
  const showPerformance = async () => { const next = await host.invoke<RuntimeStatus>("get_runtime_status"); element("performance").textContent = JSON.stringify(next.performance, null, 2); };
  element("perf-refresh").onclick = () => void action(showPerformance);
  element("perf-reset").onclick = () => void action(async () => { await host.invoke("reset_performance_session"); await showPerformance(); say("진단 기록을 지웠습니다. 설정을 닫고 버디를 테스트하세요."); });
  element("perf-export").onclick = () => void action(async () => say(`진단을 저장했습니다: ${await host.invoke<string>("export_runtime_diagnostics")}`));
  const keydown = (event: KeyboardEvent) => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") { event.preventDefault(); if (!dialog.open) void save(); }
    if (event.key === "Escape" && !dialog.open) { event.preventDefault(); requestClose(); }
  };
  const focus = () => { void refresh().catch((e) => say(String(e), true)); };
  window.addEventListener("keydown", keydown); window.addEventListener("focus", focus);
  if (host.onCloseRequested) unlisten = await host.onCloseRequested(requestClose);
  timer = window.setInterval(() => { if (!document.hidden) void refresh().catch(() => {}); }, 1500);
  await refresh(true);
}
