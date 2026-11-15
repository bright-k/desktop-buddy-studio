import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { PomodoroSettings, TimerMode } from "../../../packages/contracts/generated/index";
import { timerPresetLimit } from "../../../packages/contracts/generated/index";
import "./timer.css";

type Phase = "countdown" | "focus" | "shortBreak" | "longBreak";
type Session = {
  mode: TimerMode;
  phase: Phase;
  status: "ready" | "running" | "paused" | "done";
  durationSeconds: number;
  remainingSeconds: number;
  completed: number;
  profile: PomodoroSettings | null;
};
type TimerView = {
  revision: number;
  storageBlocked: boolean;
  mode: TimerMode;
  countdownSeconds: number;
  presets: number[];
  customPresets: boolean;
  defaultProfile: PomodoroSettings;
  personal: PomodoroSettings | null;
  showBadge: boolean;
  session: Session | null;
  outcome: { phase: Phase; skipped: boolean } | null;
  badge: { phase: Phase; label: string; time: string } | null;
  notice: string | null;
};
type Action =
  | { type: "resetStorage" }
  | { type: "selectMode"; mode: TimerMode }
  | { type: "start"; mode: TimerMode; seconds?: number; replaceRevision?: number }
  | { type: "toggle" | "skip" | "reset" | "end"; revision: number }
  | { type: "savePreset"; seconds: number }
  | { type: "setPresets"; presets: number[] | null }
  | { type: "setProfile"; profile: PomodoroSettings | null }
  | { type: "showBadge"; enabled: boolean };

const phaseName = { countdown: "일반 타이머", focus: "집중", shortBreak: "짧은 휴식", longBreak: "긴 휴식" };
const profileFields = [
  ["focusMinutes", "집중 (분)", 1, 120],
  ["shortBreakMinutes", "짧은 휴식 (분)", 1, 60],
  ["longBreakMinutes", "긴 휴식 (분)", 1, 60],
  ["rounds", "긴 휴식까지 집중 완료 (회)", 2, 8],
] as const;
const units = ["시간", "분", "초"];
const pad = (value: number) => String(value).padStart(2, "0");

function durationParts(seconds: number) {
  return [Math.floor(seconds / 3600), Math.floor(seconds / 60) % 60, seconds % 60].map(pad);
}

function durationInputs(attribute: "data-time-unit" | "data-preset-unit", context: string) {
  return `<div class="timer-units">${units.map((unit, index) =>
    `<label><input type="number" inputmode="numeric" step="1" min="0" max="${index === 0 ? 3 : 59}" aria-label="${context} ${unit}" aria-describedby="timer-error" ${attribute}>${unit}</label>`,
  ).join("")}</div>`;
}

function readDuration(fields: HTMLInputElement[]) {
  const seconds = fields.reduce((sum, field, index) => sum + Number(field.value) * [3600, 60, 1][index], 0);
  return fields.every(field => field.validity.valid) && seconds >= 1 && seconds <= 10800 ? seconds : null;
}

function time(seconds: number) {
  const hours = Math.floor(seconds / 3600);
  return [hours ? pad(hours) : null, pad(Math.floor(seconds / 60) % 60), pad(seconds % 60)].filter(part => part !== null).join(":");
}

function status(timer: Session) {
  const suffix = { ready: " 준비", paused: " 일시정지", done: " 완료", running: timer.mode === "countdown" ? " 진행 중" : " 중" };
  return phaseName[timer.phase] + suffix[timer.status];
}

function actionLabel(timer: Session) {
  if (timer.status === "running") return "타이머 일시정지";
  if (timer.status === "paused") return "타이머 재개";
  return phaseName[timer.phase] + " " + time(timer.durationSeconds) + " 시작";
}

export async function startTimerBadge(app: HTMLElement) {
  document.body.classList.add("timer-badge");
  const button = document.createElement("button");
  const label = document.createElement("span");
  const digits = document.createElement("span");
  button.type = "button";
  button.title = "타이머 창 열기";
  label.className = "badge-label";
  digits.className = "badge-digits";
  button.append(label, digits);
  app.append(button);
  const show = ({ badge }: TimerView) => {
    button.hidden = !badge;
    if (!badge) return;
    button.dataset.phase = badge.phase;
    if (badge.label !== label.textContent) label.textContent = badge.label;
    if (badge.time !== digits.textContent) digits.textContent = badge.time;
    button.setAttribute("aria-label", badge.label + " · " + badge.time + " · 타이머 창 열기");
  };
  button.hidden = true;
  button.onclick = () => { void invoke("open_timer_from_badge").catch(error => { button.title = String(error); }); };
  let received = false;
  await listen<TimerView>("timer-view", ({ payload }) => { received = true; show(payload); });
  const initial = await invoke<TimerView>("get_timer");
  if (!received) show(initial);
}

type QuickPopup = {
  id: number;
  content: { kind: "custom"; seconds: number } | { kind: "confirm"; title: string; detail: string; label: string };
  side: "above" | "below" | "side";
  tailX: number;
};
type QuickDecision = { type: "dismiss" | "confirm" } | { type: "start"; seconds: number; replaceRevision?: number };

export async function startTimerQuick(app: HTMLElement) {
  document.body.classList.add("timer-quick");
  app.innerHTML = [
    '<div class="quick-content">',
    '<form id="quick-entry" novalidate hidden><h1>사용자 지정 타이머</h1>',
    durationInputs("data-time-unit", "타이머"),
    '<div class="timer-actions"><button type="button" id="quick-cancel">취소</button><button type="submit" class="timer-primary">시작</button></div></form>',
    '<section id="quick-confirm" aria-labelledby="quick-title" hidden><h1 id="quick-title"></h1><p id="quick-detail"></p>',
    '<div class="timer-actions"><button type="button" id="quick-keep">취소</button><button type="button" id="quick-action" class="timer-primary"></button></div></section>',
    '<p id="timer-error" class="timer-error" role="alert" hidden></p>',
    '</div>',
  ].join("");
  const el = <T extends HTMLElement = HTMLElement>(id: string) => app.querySelector<T>("#" + id)!;
  const entry = el<HTMLFormElement>("quick-entry");
  const confirm = el("quick-confirm");
  const action = el("quick-action");
  const fields = [...entry.querySelectorAll<HTMLInputElement>("input")];
  let current: QuickPopup | null = null;
  let busy = false;
  let replacement: { seconds: number; revision: number } | null = null;

  function message(text = "") {
    const error = el("timer-error");
    error.textContent = text;
    error.hidden = !text;
  }
  function showEntry() {
    replacement = null;
    confirm.hidden = true;
    entry.hidden = false;
    fields[1].focus();
    fields[1].select();
  }
  function showConfirmation(title: string, detail: string, label: string) {
    entry.hidden = true;
    confirm.hidden = false;
    el("quick-title").textContent = title;
    el("quick-detail").textContent = detail;
    action.textContent = label;
    action.classList.toggle("quick-end", label === "종료");
    el("quick-keep").focus();
  }
  function show(popup: QuickPopup) {
    document.body.dataset.side = popup.side;
    app.style.setProperty("--quick-tail", popup.tailX + "px");
    if (current?.id === popup.id) return;
    current = popup;
    busy = false;
    replacement = null;
    message();
    if (popup.content.kind === "custom") {
      const parts = durationParts(popup.content.seconds);
      fields.forEach((field, index) => { field.value = parts[index]; });
      showEntry();
    } else {
      showConfirmation(popup.content.title, popup.content.detail, popup.content.label);
    }
  }
  async function run(work: (popup: QuickPopup) => Promise<void>) {
    if (busy || !current) return;
    const popup = current;
    busy = true;
    message();
    try {
      await work(popup);
    } catch (error) {
      if (current?.id === popup.id) message(String(error));
    } finally {
      if (current?.id === popup.id) busy = false;
    }
  }
  function finish(popup: QuickPopup, decision: QuickDecision) {
    return invoke<void>("finish_timer_quick", { id: popup.id, decision });
  }
  function cancel() {
    if (busy) return;
    if (replacement) {
      message();
      showEntry();
    } else {
      void run(popup => finish(popup, { type: "dismiss" }));
    }
  }

  entry.oninput = () => message();
  entry.onsubmit = event => {
    event.preventDefault();
    if (busy) return;
    const seconds = readDuration(fields);
    if (seconds === null) {
      message("시간은 1초부터 3시간까지 설정할 수 있습니다.");
      (fields.find(field => !field.validity.valid) ?? fields[1]).focus();
      return;
    }
    void run(async popup => {
      const view = await invoke<TimerView>("get_timer");
      if (current?.id !== popup.id) return;
      if (view.session && view.session.status !== "done") {
        replacement = { seconds, revision: view.revision };
        showConfirmation("타이머를 바꿀까요?", "현재 타이머를 종료하고 " + time(seconds) + " 타이머를 시작합니다.", "종료하고 시작");
      } else {
        await finish(popup, { type: "start", seconds });
      }
    });
  };
  el("quick-cancel").onclick = el("quick-keep").onclick = cancel;
  action.onclick = () => {
    const next = replacement;
    void run(popup => finish(popup, next
      ? { type: "start", seconds: next.seconds, replaceRevision: next.revision }
      : { type: "confirm" }));
  };
  document.addEventListener("keydown", event => {
    if (event.key === "Escape") {
      event.preventDefault();
      cancel();
    }
  });
  await listen<QuickPopup>("timer-quick", ({ payload }) => show(payload));
  const initial = await invoke<QuickPopup | null>("get_timer_quick");
  if (initial && !current) show(initial);
}

export async function startTimer(app: HTMLElement) {
  document.body.classList.add("timer-window");
  app.innerHTML = [
    '<section id="timer-storage" hidden><h2>타이머 저장 파일 보호</h2><p id="timer-storage-notice" role="status"></p><button type="button" id="reset-storage">원본 보관 후 초기화…</button></section>',
    '<nav class="timer-modes" aria-label="타이머 모드"><button id="mode-count" type="button">일반 타이머</button><button id="mode-pomo" type="button">뽀모도로</button></nav>',
    '<div id="timer-other" class="timer-status" hidden><span id="other-status"></span><button id="go-current" type="button">진행 화면 보기</button></div>',
    '<div id="count-setup" class="timer-stack">',
    '<div class="timer-stack"><div class="timer-row"><strong>시간 바로가기</strong><button id="manage-times" type="button" aria-controls="preset-editor" aria-expanded="false">관리</button></div><div id="saved-buttons" class="timer-actions"></div><div id="preset-source" class="timer-note"></div></div>',
    '<div id="preset-editor" class="timer-stack" hidden><div id="preset-rows" class="timer-stack"></div><button id="add-preset" type="button">＋ 바로가기</button><div id="preset-error" class="timer-error" role="alert" hidden></div><div class="timer-actions"><button id="save-presets" class="timer-primary" type="button">목록 저장</button><button id="cancel-presets" type="button">취소</button><button id="restore-presets" type="button">Studio 기본 목록 복원</button></div></div>',
    '<div id="count-entry" class="timer-stack"><fieldset class="timer-time"><legend>이번 시간</legend>',
    durationInputs("data-time-unit", "이번 시간"),
    '</fieldset><div class="timer-start"><button id="start-count" class="timer-primary" type="button">시작</button><button id="save-time" type="button">이 시간 저장</button></div></div></div>',
    '<div id="timer-stage" class="timer-stack" hidden><div class="timer-face" id="timer-face"><strong id="stage-name"></strong><div class="timer-digits" id="timer-digits" role="timer" aria-label="남은 시간" aria-live="off"></div><progress id="timer-progress" max="100" aria-label="단계 진행률"></progress><div id="timer-cycle"></div></div>',
    '<div class="timer-actions"><button id="timer-toggle" class="timer-primary" type="button"></button><button id="timer-skip" type="button" hidden></button></div><div id="timer-next" class="timer-note" hidden></div></div>',
    '<div id="timer-outcome" class="timer-status" role="status" hidden></div>',
    '<details id="timer-profile" hidden><summary>뽀모도로 설정 <span id="profile-summary" class="timer-note"></span></summary>',
    '<div class="timer-profile-fields">',
    ...profileFields.map(([key, name, min, max]) => '<label>' + name + '<input id="timer-' + key + '" type="number" step="1" min="' + min + '" max="' + max + '" required></label>'),
    '</div><div class="timer-actions"><button id="save-profile" class="timer-primary" type="button">설정 저장</button><button id="restore-profile" type="button">Studio 기본값 복원</button></div><div class="timer-note" id="profile-note" role="status"></div></details>',
    '<details id="timer-more"><summary>더 보기</summary><label class="timer-checkbox"><input id="show-timer-badge" type="checkbox">버디 옆에 시간 표시</label><div id="session-actions" class="timer-actions" hidden><button id="reset-timer" type="button">현재 단계 초기화</button><button id="end-timer" type="button">세션 종료</button></div></details>',
    '<div id="timer-error" class="timer-error" role="alert" hidden></div><div id="timer-note" class="timer-note" role="status" hidden></div>',
    '<dialog id="timer-confirm" aria-labelledby="timer-confirm-title"><h2 id="timer-confirm-title"></h2><p id="timer-confirm-detail"></p><div class="timer-actions"><button id="timer-cancel" type="button" autofocus>취소</button><button id="timer-confirm-action" type="button" class="timer-primary"></button></div></dialog>',
  ].join("");
  const el = <T extends HTMLElement = HTMLElement>(id: string) => app.querySelector<T>("#" + id)!;
  const fields = [...app.querySelectorAll<HTMLInputElement>("[data-time-unit]")];
  const profileInput = (key: keyof PomodoroSettings) => el<HTMLInputElement>("timer-" + key);
  const dialog = el<HTMLDialogElement>("timer-confirm");
  const put = (id: string, value: string) => {
    const target = el(id);
    if (target.textContent !== value) target.textContent = value;
  };
  function message(id: "timer-error" | "timer-note", value = "") {
    put(id, value);
    el(id).hidden = !value;
  }

  let current = await invoke<TimerView>("get_timer");
  let view = current.session?.mode ?? current.mode;
  let dirtyDuration = false;
  let managing = false;
  let presetKey = "";
  let busy = false;
  let confirmAction: (() => void) | undefined;

  const personalProfile = () => current.personal ?? current.defaultProfile;
  const activeSession = () => current.session?.status !== "done" ? current.session : null;
  function setDuration(seconds: number) {
    const values = durationParts(seconds);
    fields.forEach((field, index) => { field.value = values[index]; });
  }

  function setProfile(profile: PomodoroSettings) {
    for (const [key] of profileFields) profileInput(key).value = String(profile[key]);
  }

  function previewSession(profile: PomodoroSettings): Session {
    return { mode: "pomodoro", phase: "focus", status: "ready", completed: 0, profile,
      durationSeconds: profile.focusMinutes * 60, remainingSeconds: profile.focusMinutes * 60 };
  }

  function nextPhase(session: Session, profile: PomodoroSettings): [Phase, number] {
    if (session.phase !== "focus") return ["focus", profile.focusMinutes * 60];
    return session.completed + 1 >= profile.rounds
      ? ["longBreak", profile.longBreakMinutes * 60]
      : ["shortBreak", profile.shortBreakMinutes * 60];
  }

  function renderPresets() {
    const key = JSON.stringify([current.presets, current.customPresets, managing]);
    if (key === presetKey) return;
    presetKey = key;
    el("preset-editor").hidden = !managing;
    el("count-entry").hidden = managing;
    el("saved-buttons").hidden = managing || !current.presets.length;
    el("preset-source").hidden = managing;
    put("preset-source", !current.presets.length ? "등록된 시간이 없습니다." : current.customPresets ? "개인 목록" : "Studio 기본 목록");
    el<HTMLButtonElement>("restore-presets").disabled = !current.customPresets;
    put("manage-times", managing ? "목록 편집 중" : "관리");
    el<HTMLButtonElement>("manage-times").disabled = managing;
    el("manage-times").setAttribute("aria-expanded", String(managing));
    el("saved-buttons").replaceChildren(...current.presets.map(seconds => {
      const button = document.createElement("button");
      button.type = "button";
      button.textContent = time(seconds) + " 시작";
      button.onclick = () => requestStart("countdown", seconds);
      return button;
    }));
  }

  function presetRows() {
    return [...el("preset-rows").querySelectorAll("fieldset")].map(row => [...row.querySelectorAll<HTMLInputElement>("[data-preset-unit]")]);
  }

  function presetError(value = "") {
    put("preset-error", value);
    el("preset-error").hidden = !value;
  }

  function renderPresetEditor(rows: string[][]) {
    el("preset-rows").replaceChildren(...rows.map((values, index) => {
      const row = document.createElement("fieldset");
      row.className = "timer-time timer-preset";
      row.innerHTML = `<legend>바로가기 ${index + 1}</legend>${durationInputs("data-preset-unit", `바로가기 ${index + 1}`)}
        <div class="timer-actions"><button type="button" data-preset-order="-1" ${index === 0 ? "disabled" : ""}>위로</button>
        <button type="button" data-preset-order="1" ${index === rows.length - 1 ? "disabled" : ""}>아래로</button>
        <button type="button" data-preset-remove>삭제</button></div>`;
      row.querySelectorAll<HTMLInputElement>("input").forEach((input, i) => {
        input.value = values[i];
        input.setAttribute("aria-describedby", "preset-error");
        input.oninput = () => presetError();
      });
      row.querySelectorAll<HTMLButtonElement>("button").forEach(button => { button.onclick = () => {
        const draft = presetRows().map(fields => fields.map(field => field.value));
        let focusIndex = index;
        if (button.hasAttribute("data-preset-remove")) draft.splice(index, 1);
        else {
          focusIndex += Number(button.dataset.presetOrder);
          [draft[index], draft[focusIndex]] = [draft[focusIndex], draft[index]];
        }
        renderPresetEditor(draft);
        (presetRows()[Math.min(focusIndex, draft.length - 1)]?.[0] ?? el("add-preset")).focus();
      }; });
      return row;
    }));
    el<HTMLButtonElement>("add-preset").disabled = rows.length >= timerPresetLimit;
    presetError();
  }

  function render() {
    app.dataset.storageBlocked = String(current.storageBlocked);
    el("timer-storage").hidden = !current.storageBlocked;
    if (current.storageBlocked) {
      put("timer-storage-notice", current.notice ?? "");
      return;
    }
    el("mode-count").setAttribute("aria-pressed", String(view === "countdown"));
    el("mode-pomo").setAttribute("aria-pressed", String(view === "pomodoro"));
    const active = activeSession();
    const other = active && active.mode !== view ? active : null;
    el("timer-other").hidden = !other;
    if (other) put("other-status", status(other) + " · " + time(other.remainingSeconds));
    const own = current.session?.mode === view ? current.session : null;
    const countSetup = view === "countdown" && (!own || own.status === "done");
    el("count-setup").hidden = !countSetup;
    el("timer-stage").hidden = countSetup;
    const draft = readDuration(fields);
    put("start-count", own?.status === "done" && own.durationSeconds === draft ? "다시 시작" : "시작");
    const profile = personalProfile();
    if (!countSetup) {
      const timer = own ?? previewSession(profile);
      put("stage-name", status(timer));
      put("timer-digits", time(timer.remainingSeconds));
      el("timer-face").dataset.phase = timer.phase;
      const progress = el<HTMLProgressElement>("timer-progress");
      progress.hidden = timer.status === "ready";
      progress.value = (1 - timer.remainingSeconds / timer.durationSeconds) * 100;
      el("timer-cycle").hidden = !timer.profile;
      el("timer-next").hidden = !timer.profile;
      if (timer.profile) {
        put("timer-cycle", "집중 완료 " + timer.completed + " / " + timer.profile.rounds);
        const [phase, seconds] = nextPhase(timer, timer.profile);
        put("timer-next", "다음 단계 · " + phaseName[phase] + " " + time(seconds));
      }
      put("timer-toggle", actionLabel(timer));
      el("timer-skip").hidden = !own || !timer.profile || (timer.phase === "focus" && timer.status === "ready");
      put("timer-skip", timer.phase === "focus" ? "집중 건너뛰기" : "휴식 건너뛰고 집중 시작");
    }
    const outcome = current.outcome;
    el("timer-outcome").hidden = !outcome || (outcome.phase === "countdown") !== (view === "countdown");
    if (outcome) put("timer-outcome", phaseName[outcome.phase] + (outcome.skipped ? " 건너뜀" : " 완료"));
    el("timer-profile").hidden = view !== "pomodoro";
    const sessionProfile = active?.profile;
    const nextSession = sessionProfile && profileFields.some(([key]) => profile[key] !== sessionProfile[key]);
    const prefix = nextSession ? "다음 세션 · " : current.personal ? "개인 · " : "";
    put("profile-summary", prefix + [profile.focusMinutes, profile.shortBreakMinutes, profile.longBreakMinutes].join(" / ") + "분 · " + profile.rounds + "회");
    el<HTMLInputElement>("show-timer-badge").checked = current.showBadge;
    el("session-actions").hidden = !active || active.mode !== view;
    put("reset-timer", view === "countdown" ? "타이머 초기화" : "현재 단계 초기화");
    put("end-timer", view === "countdown" ? "타이머 종료" : "세션 종료");
    renderPresets();
  }

  function update(next: TimerView) {
    if (next.revision < current.revision) return;
    const oldProfile = personalProfile();
    const untouched = profileFields.every(([key]) => profileInput(key).valueAsNumber === oldProfile[key]);
    const changed = next.countdownSeconds !== current.countdownSeconds;
    current = next;
    if (!dirtyDuration && changed) setDuration(current.countdownSeconds);
    if (untouched) setProfile(personalProfile());
    if (current.notice && !current.storageBlocked) message("timer-error", current.notice);
    render();
  }

  async function apply(action: Action) {
    if (busy) return false;
    busy = true;
    message("timer-error");
    try {
      update(await invoke<TimerView>("timer_action", { action }));
      return true;
    } catch (error) {
      await invoke<TimerView>("get_timer").then(update).catch(() => {});
      message("timer-error", String(error));
      return false;
    } finally { busy = false; }
  }

  function openCurrent() {
    view = activeSession()?.mode ?? current.mode;
    render();
    if (el("timer-stage").hidden) (managing ? presetRows()[0]?.[0] ?? el("add-preset") : fields[1]).focus();
    else el("timer-toggle").focus();
  }

  function confirm(title: string, detail: string, label: string, action: () => void) {
    if (dialog.open) return;
    confirmAction = action;
    put("timer-confirm-title", title);
    put("timer-confirm-detail", detail);
    put("timer-confirm-action", label);
    dialog.showModal();
    el("timer-cancel").focus();
  }

  function requestStart(mode: TimerMode, seconds?: number) {
    if (busy) return;
    const active = activeSession();
    const start = async (replaceRevision?: number) => {
      if (!await apply({ type: "start", mode, seconds, replaceRevision })) return;
      view = mode;
      if (mode === "countdown") {
        dirtyDuration = false;
        setDuration(current.countdownSeconds);
      }
      message("timer-note");
      render();
      el("timer-toggle").focus();
    };
    if (active) {
      const revision = current.revision;
      confirm("현재 타이머를 종료할까요?", status(active) + " · " + time(active.remainingSeconds) + ". 전환하면 새 타이머를 시작합니다.",
        "종료하고 시작", () => { void start(revision); });
    } else void start();
  }

  function enterDuration(save: boolean) {
    const seconds = readDuration(fields);
    if (seconds === null) {
      message("timer-error", "시간은 0~3, 분·초는 0~59로 입력하세요. 전체 범위는 1초~3시간입니다.");
      fields.find(field => !field.validity.valid)?.focus();
      return;
    }
    if (!save) {
      requestStart("countdown", seconds);
      return;
    }
    void apply({ type: "savePreset", seconds }).then(ok => { if (ok) message("timer-note", time(seconds) + " 저장됨"); });
  }

  function changeView(mode: TimerMode) {
    view = mode;
    message("timer-error");
    message("timer-note");
    render();
    if (!activeSession() && current.mode !== mode) void apply({ type: "selectMode", mode });
  }

  el("mode-count").onclick = () => changeView("countdown");
  el("mode-pomo").onclick = () => changeView("pomodoro");
  el("go-current").onclick = openCurrent;
  el("start-count").onclick = () => enterDuration(false);
  el("save-time").onclick = () => enterDuration(true);
  el("manage-times").onclick = () => {
    managing = true;
    renderPresets();
    renderPresetEditor(current.presets.map(durationParts));
    (presetRows()[0]?.[0] ?? el("add-preset")).focus();
  };
  function closePresetEditor() {
    managing = false;
    renderPresets();
    el("manage-times").focus();
  }
  el("cancel-presets").onclick = closePresetEditor;
  el("add-preset").onclick = () => {
    const rows = presetRows();
    const used = rows.map(readDuration);
    const seconds = [readDuration(fields), ...Array.from({ length: timerPresetLimit }, (_, i) => (i + 1) * 60)]
      .find(seconds => seconds !== null && !used.includes(seconds));
    if (seconds == null) return;
    renderPresetEditor([...rows.map(row => row.map(field => field.value)), durationParts(seconds)]);
    presetRows().at(-1)?.[0].focus();
  };
  el("save-presets").onclick = () => {
    const rows = presetRows();
    const presets = rows.map(readDuration);
    const invalid = presets.indexOf(null);
    if (invalid !== -1) {
      presetError("각 바로가기 시간을 1초부터 3시간 사이로 입력하세요.");
      (rows[invalid].find(field => !field.validity.valid) ?? rows[invalid][0]).focus();
      return;
    }
    if (new Set(presets).size !== presets.length) {
      presetError("같은 시간은 하나만 등록할 수 있습니다.");
      return;
    }
    void apply({ type: "setPresets", presets: presets as number[] }).then(ok => {
      if (ok) {
        closePresetEditor();
        message("timer-note", "바로가기 목록 저장됨");
      }
    });
  };
  el("restore-presets").onclick = () => {
    void apply({ type: "setPresets", presets: null }).then(ok => {
      if (ok) {
        closePresetEditor();
        message("timer-note", "Studio 기본 목록 복원됨");
      }
    });
  };
  for (const field of fields) {
    field.oninput = () => { dirtyDuration = true; message("timer-error"); render(); };
    field.onkeydown = event => { if (event.key === "Enter") { event.preventDefault(); enterDuration(false); } };
  }
  el("timer-toggle").onclick = () => {
    if (current.session?.mode === view && current.session.status !== "done") {
      void apply({ type: "toggle", revision: current.revision });
    } else requestStart(view);
  };
  el("timer-skip").onclick = () => { void apply({ type: "skip", revision: current.revision }); };
  el("reset-timer").onclick = () => { void apply({ type: "reset", revision: current.revision }); };
  el("end-timer").onclick = () => {
    const revision = current.revision;
    confirm("세션을 종료할까요?", "현재 타이머와 이번 세션의 집중 횟수를 초기화합니다.", "종료",
      () => { void apply({ type: "end", revision }); });
  };
  el("timer-cancel").onclick = () => dialog.close();
  el("reset-storage").onclick = () => {
    confirm("타이머 저장 파일을 초기화할까요?", "원본은 별도 파일로 보관하고 Studio 기본값으로 다시 시작합니다.",
      "보관하고 초기화", () => { void apply({ type: "resetStorage" }).then(ok => { if (ok) openCurrent(); }); });
  };
  el("timer-confirm-action").onclick = () => {
    const action = confirmAction;
    dialog.close();
    action?.();
  };
  dialog.addEventListener("close", () => { confirmAction = undefined; });
  el<HTMLInputElement>("show-timer-badge").onchange = event => {
    void apply({ type: "showBadge", enabled: (event.target as HTMLInputElement).checked });
  };

  async function saveProfile(profile: PomodoroSettings | null) {
    if (!await apply({ type: "setProfile", profile })) return;
    setProfile(personalProfile());
    const label = profile ? "개인 설정 저장됨" : "Studio 기본값 복원됨";
    put("profile-note", label + (activeSession()?.mode === "pomodoro" ? " · 다음 새 세션부터 적용" : ""));
  }
  el("save-profile").onclick = () => {
    const invalid = profileFields.find(([key]) => !profileInput(key).validity.valid);
    if (invalid) { profileInput(invalid[0]).reportValidity(); return; }
    void saveProfile(Object.fromEntries(profileFields.map(([key]) => [key, profileInput(key).valueAsNumber])) as PomodoroSettings);
  };
  el("restore-profile").onclick = () => { void saveProfile(null); };

  setDuration(current.countdownSeconds);
  setProfile(personalProfile());
  if (current.notice && !current.storageBlocked) message("timer-error", current.notice);
  render();
  await listen<TimerView>("timer-view", ({ payload }) => update(payload));
  await listen("timer-focus", openCurrent);
  update(await invoke<TimerView>("get_timer"));
  openCurrent();
}
