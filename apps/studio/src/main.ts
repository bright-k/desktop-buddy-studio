import type {
  Project,
  Character,
  Binding,
  Animation,
  CursorDirection,
  CursorPose,
  IdleCursorMode,
  IdleGroup,
  Effect,
  Interruption,
  FrameRegion,
  CharacterState,
  KeyChord,
  KeyModifier,
  KeyStep,
  MacosDmg,
  PointerInput,
  RenderPlan,
  Resource,
  SoundResource,
  Target,
  TimerPhase,
} from "../../../packages/contracts/generated/index";
import { animationScaleLimits, buddyHoldMs, defaultTimerPresets, supportedPhysicalKeys, timerPresetLimit } from "../../../packages/contracts/generated/index";
import { CharacterRenderer, horizontalAnchorExtent } from "../../../packages/renderer/src/index";
import { ReactionAudio } from "../../../packages/renderer/src/audio";
import macDmgPreset from "../../../packaging/macos/defaults/layout.json";
import { readRoute, writeRoute, type StudioRoute } from "./route";
import "./style.css";
type Meta = {
  id: string;
  packageId: string;
  legacyIds?: string[];
  revision: string;
  name: string;
  slug: string;
  builtSlug?: string;
  identifier: string;
  archived: boolean;
  updatedAt: string;
};
type ResourceFile = {
  name: string;
  originalName: string;
  bytes: number;
  width?: number;
  height?: number;
  frames?: number;
  animated?: boolean;
};
type MotionPreview = { bindingId: string; revision: string; seed: number; starts: number };
type TypingStoppedTrigger = Extract<Binding["trigger"], { type: "typingStopped" }>;
type TypingPreview = { bindingId: string; startAtMs?: number; quietMessage?: string };
type SimulationFrame = { atMs: number; state: CharacterState; bindingId?: string | null };
type StudioCharacter = Omit<Character, "resources" | "sounds" | "animations"> & {
  resources: Record<string, Resource>;
  sounds: Record<string, SoundResource>;
  animations: Record<string, Animation & { regions: FrameRegion[] }>;
};
type Document = {
  meta: Meta;
  project: Project;
  character: StudioCharacter;
  license: string;
  library: Record<string, ResourceFile>;
  renderPlan: RenderPlan;
  images: string[];
  sequences: (KeyStep[] | null)[];
  sequenceAlternatives?: KeyStep[][][];
  contentHash: string;
  idleHeightDipAtScaleOne?: number;
  draft?: {
    baseRevision: string;
    draftRevision: string;
    project: Project;
    character: StudioCharacter;
    license: string;
    updatedAt: string;
  } | null;
};
type Job = {
  id: string;
  productIdentifier?: string;
  provider?: string;
  target?: string;
  revision: string;
  status: string;
  createdAt: string;
  development: boolean;
  logs: string;
  error?: string;
  remoteConclusion?: string;
  downloads?: { name: string; bytes?: number }[];
  result?: { directory: string };
  github?: { repository: string; runId?: number; runUrl?: string };
  platforms?: Record<string, { status: string; conclusion?: string; imported?: boolean; importError?: string; steps?: {name: string; status: string; conclusion?: string}[] }>;
};
type GitHubState = { repository: string; connected: boolean; ready: boolean; private?: boolean; login?: string; defaultBranch?: string; workflowPresent?: boolean; credentialSource?: string; error?: string };
let github: GitHubState = { repository: "", connected: false, ready: false }, githubError = "";
const app = document.querySelector<HTMLElement>("#app")!;
let projects: Meta[] = [],
  doc: Document | null = null,
  saved: Document | null = null,
  jobs: Job[] = [],
  tab = "overview",
  selectedClip = "idle",
  selectedBinding = 0,
  selectedIdleGroup = "",
  dirty = false,
  busy = false,
  archived = false;
let idleGroupDraft: IdleGroup | undefined;
let selectedIdleDirection: CursorDirection = "center";
let selectedIdlePhase: TimerPhase | undefined;
let renderer: CharacterRenderer | undefined,
  previewDocument: Document | undefined,
  previewClip = "",
  previewWork = "",
  simulationTimers: number[] = [],
  cropImage: HTMLImageElement | undefined,
  crop: FrameRegion = { x: 0, y: 0, width: 256, height: 256 };
let gifFrameTimer = 0, gifFrameRequest = 0;
let scalePreviews: CharacterRenderer[] = [];
let config: { dataRoot: string; target: Target; development: boolean } = { dataRoot: "", target: "macos-arm64", development: false },
  messageTimer = 0,
  currentEpoch = 1,
  pollTimer = 0;
let draftRevision: string | null = null,
  draftState: "idle" | "pending" | "saving" | "saved" | "failed" = "idle",
  draftTimer = 0,
  draftChain: Promise<unknown> = Promise.resolve(),
  draftContent = "",
  restoredDraft = false,
  validationError = "",
  projectRequest = 0,
  simulationRequest = 0;
let connectionIssue = "", polling = false, projectLoading = false;
let motionPreviewRevision = "";
const motionPreviews = new Map<string, MotionPreview>();
let editorWork = "";
const operationStatus = document.createElement("div");
operationStatus.className = "operation-status";
operationStatus.setAttribute("role", "status");
operationStatus.innerHTML = '<i class="operation-spinner" aria-hidden="true"></i><span></span>';
operationStatus.hidden = true;
app.after(operationStatus);

function updateOperationStatus() {
  const message = editorWork || (projectLoading ? "프로젝트와 미리보기를 불러오는 중…" : "");
  operationStatus.querySelector("span")!.textContent = message;
  operationStatus.hidden = !message;
}
function setEditorBusy(message = "") {
  editorWork = message;
  busy = !!message;
  app.inert = busy;
  if (busy) app.setAttribute("aria-busy", "true");
  else app.removeAttribute("aria-busy");
  updateOperationStatus();
}
const esc = (value: unknown) =>
  String(value ?? "").replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ]!,
  );
const prettySize = (n: number) =>
  n < 1024 * 1024
    ? `${Math.ceil(n / 1024)} KiB`
    : `${(n / (1024 * 1024)).toFixed(1)} MiB`;
const date = (value: string) =>
  new Date(value).toLocaleString("ko-KR", {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
const api = async <T = any>(
  path: string,
  options: RequestInit = {},
): Promise<T> => {
  let response: Response;
  try { response = await fetch(`/api/${path}`, {
    ...options,
    headers: {
      ...(options.body && typeof options.body === "string"
        ? { "Content-Type": "application/json" }
        : {}),
      ...options.headers,
    },
  });
  } catch (error) {
    setConnection("Studio와 연결이 끊겼습니다. 실행 중인 터미널을 확인하세요. 편집 내용은 이 화면에 유지됩니다.");
    throw error;
  }
  const value = await response.json();
  if (response.status === 401 && value.error?.includes("Studio")) setConnection("Studio 연결이 만료됐습니다. 터미널에 표시된 최신 실행 주소를 다시 여세요. 저장하지 않은 편집은 먼저 JSON으로 내보낼 수 있습니다.");
  else setConnection("");
  if (!response.ok) throw new Error(value.error ?? "요청에 실패했습니다.");
  for (const character of [value.character, value.draft?.character].filter(
    Boolean,
  )) {
    character.resources ??= {};
    character.sounds ??= {};
    for (const a of Object.values(character.animations) as Animation[])
      a.regions ??= [];
  }
  return value;
};
const projectApi = async <T = any>(action = "", options: RequestInit = {}) =>
  api<T>(`projects/${doc!.meta.id}${action ? `/${action}` : ""}`, options);
const post = (value: unknown) => ({
  method: "POST",
  body: JSON.stringify(value),
});
const sourceUrl = (image: string) =>
  `/api/projects/${doc!.meta.id}/source/character/${image.split("/").map(encodeURIComponent).join("/")}`;
const resourceUrl = (file: string) =>
  `/api/projects/${doc!.meta.id}/resources/${encodeURIComponent(file)}`;
function setConnection(message: string) {
  connectionIssue = message;
  let banner = document.querySelector<HTMLElement>("#connection-banner");
  if (!banner && document.querySelector(".workspace")) {
    banner = document.createElement("div"); banner.id = "connection-banner"; banner.className = "connection-banner"; banner.setAttribute("role", "alert");
    banner.innerHTML = '<span></span><button class="small" data-action="export-draft">편집 JSON 저장</button><button class="small" data-action="retry-connection">다시 연결</button>';
    app.prepend(banner);
  }
  if (banner) {
    banner.hidden = !message;
    const text = banner.querySelector("span")!;
    if (text.textContent !== message) text.textContent = message;
    banner.querySelector<HTMLElement>('[data-action="export-draft"]')!.hidden = !doc;
  }
}
function syncRoute(push = false) {
  if (!doc) return;
  const url = writeRoute(new URL(location.href), {
    projectId: doc.meta.id, tab: tab as StudioRoute["tab"], clip: selectedClip,
    bindingId: doc.project.bindings[selectedBinding]?.id ?? null,
  });
  if (url.href !== location.href) history[push ? "pushState" : "replaceState"](null, "", url);
}
function toast(message: string, error = false) {
  const el = document.querySelector<HTMLElement>("#toast")!;
  el.textContent = message;
  el.className = `toast show ${error ? "error" : ""}`;
  clearTimeout(messageTimer);
  messageTimer = window.setTimeout(
    () => el.classList.remove("show"),
    error ? 12000 : 4500,
  );
}
function draftPayload() {
  return {
    revision: doc!.meta.revision,
    project: doc!.project,
    character: doc!.character,
    license: doc!.license,
  };
}
function updateEditingStatus() {
  document.title = doc
    ? `${doc.project.product.name}${dirty ? " •" : ""} · Buddy Studio`
    : "Desktop Buddy Studio";
  const timing = document.querySelector<HTMLElement>(".timing-highlight");
  if (timing && doc) {
    const paragraph = timing.querySelector("p"),
      number = timing.querySelector("strong");
    if (paragraph)
      paragraph.textContent = `마지막 일반 키 입력 후 ${doc.project.defaults.timing.idleTimeoutMs}ms에 속도 애니메이션을 끝냅니다.`;
    if (number)
      number.innerHTML = `${esc(doc.project.defaults.timing.idleTimeoutMs)}<small>ms</small>`;
  }
  const status = document.querySelector<HTMLElement>("#save-state");
  if (status) {
    status.textContent = !dirty
      ? "모든 변경 저장됨"
      : draftState === "saved"
        ? "임시 저장됨 · 변경 미반영"
        : draftState === "saving"
          ? "임시 저장 중"
          : draftState === "failed"
            ? "임시 저장 실패"
            : "저장하지 않은 변경";
    status.className = `save-state ${dirty ? "dirty" : ""}`;
  }
  for (const button of document.querySelectorAll<HTMLButtonElement>(
    '[data-action="save"]',
  ))
    button.disabled = !dirty || !!doc?.meta.archived;
  const notice = document.querySelector<HTMLElement>("#preview-notice");
  if (notice)
    notice.textContent = dirty
      ? "변경사항을 저장하면 이 미리보기에 반영됩니다."
      : "마지막으로 저장한 구성을 보여주고 있습니다.";
  const button = document.querySelector<HTMLElement>("#preview-save");
  if (button) button.hidden = !dirty;
  const cancel = document.querySelector<HTMLElement>("#cancel-edit");
  if (cancel) cancel.hidden = !dirty;
}
function markDirty() {
  dirty = true;
  validationError = "";
  draftState = "pending";
  document.querySelector<HTMLElement>("#validation-panel")?.remove();
  updateEditingStatus();
  clearTimeout(draftTimer);
  draftTimer = window.setTimeout(() => void flushDraft().catch(() => {}), 800);
}
async function flushDraft() {
  clearTimeout(draftTimer);
  if (!doc || !dirty || doc.meta.archived) return;
  const document = doc,
    content = JSON.stringify(draftPayload()),
    id = doc.meta.id;
  const task = draftChain
    .catch(() => {})
    .then(async () => {
      if (doc !== document || content === draftContent) return;
      draftState = "saving";
      updateEditingStatus();
      try {
        const result = await api<any>(`projects/${id}/draft`, {
          method: "PUT",
          body: JSON.stringify({ ...JSON.parse(content), draftRevision }),
        });
        if (doc === document) {
          draftRevision = result.draftRevision;
          draftContent = content;
          draftState =
            content === JSON.stringify(draftPayload()) ? "saved" : "pending";
          updateEditingStatus();
        }
      } catch (error) {
        if (doc === document) {
          draftState = "failed";
          updateEditingStatus();
        }
        throw error;
      }
    });
  draftChain = task;
  return task;
}
function validationHint(message: string) {
  if (message.includes("unsupported by the selected layout"))
    return "키 배열과 키워드를 확인하세요. 한글은 한글 두벌식을 선택하고, 영문 QWERTY에는 영문자·숫자·공백을 쓰세요. 특수문자는 두 배열 모두 지원하지 않습니다.";
  if (message.includes("duplicate compiled key sequence"))
    return "같은 물리 키 순서를 사용하는 키워드가 있습니다. 키워드 목록과 다른 이벤트의 키워드를 확인하세요.";
  if (message.includes("project decoded images exceed"))
    return "이미지를 프레임으로 펼쳤을 때의 용량이 프로젝트 한도를 넘었습니다. 이미지 크기나 GIF 프레임 수를 줄인 파일로 교체하세요.";
  if (message.includes("invalid typing rate range"))
    return "입력 속도는 0.5~1000 키/초로 지정하세요. 최대 속도를 지정했다면 최소 속도보다 커야 합니다.";
  if (message.includes("$.") && /\.fps\b/.test(message))
    return "재생 속도(FPS)를 1~30 사이로 설정해주세요.";
  if (message.includes("ranges must not overlap"))
    return "입력 속도 구간이 겹칩니다. 각 이벤트의 최소·최대 속도를 조정해주세요.";
  if (message.includes("one-shot"))
    return "일회성 이벤트에는 반복 재생이 꺼진 애니메이션을 선택하세요. 목록이 비어 있으면 애니메이션 탭에서 먼저 준비하세요.";
  if (message.includes("sustain"))
    return "입력 속도·키 유지·드래그 동작은 반복 재생 애니메이션에 연결해주세요.";
  if (message.includes("INVALID_ICON"))
    return "아이콘은 512~1024px 정사각형 PNG여야 합니다.";
  if (message.includes("resource names must contain"))
    return "리소스 이름은 1~80자로 입력하세요. 이름을 지우면 원본 이름을 사용합니다.";
  if (message.includes("$.idle.directions") || message.includes("$.defaultAnimation"))
    return "대기 상태에는 반복 재생 동작이나 이미지·GIF 리소스를 연결하세요.";
  if (message.includes("$.idleGroups"))
    return message.split("\n").find(line => line.includes("$.idleGroups"))?.split("[INVALID_PROJECT] ").at(-1)
      ?? "대기 모습 묶음의 적용 상태와 연결한 리소스를 확인하세요.";
  if (message.includes("$.cursorFollow"))
    return "멈춤 거리와 거리별 이동 속도를 확인하세요. 구간에는 반복 애니메이션을 연결할 수 있습니다.";
  if (message.includes("$.defaults.displayHeightDip"))
    return "대기 애니메이션의 표시 높이는 16~1024 DIP로 설정하세요.";
  if (message.includes("$.build.macosDmg.guidePosition"))
    return "가이드 아이콘이 창 안에 있고 다른 아이콘과 겹치지 않도록 X·Y를 조정하거나 자동 배치를 선택하세요.";
  if (message.includes("$.defaults.timing"))
    return "입력 중단·평가·안정 시간의 허용 범위를 확인해주세요. 평가 간격은 입력 중단 시간 이하여야 합니다.";
  if (message.includes("Another window") || message.includes("다른 창"))
    return "다른 창에서 수정되었습니다. 현재 편집 JSON을 내려받아 보관한 뒤 프로젝트를 다시 여세요.";
  return "자세한 오류에서 문제가 있는 항목을 확인하세요. 수정한 내용은 편집 화면에 남아 있습니다.";
}
function validationPanel() {
  return validationError
    ? `<div id="validation-panel" class="validation-panel" role="alert" tabindex="-1"><strong>저장하기 전에 확인해주세요</strong><p>${esc(validationHint(validationError))}</p><details><summary>자세한 오류 보기</summary><pre>${esc(validationError)}</pre></details><button class="text-button" data-action="export-draft">현재 편집 JSON 내려받기</button></div>`
    : "";
}
function showValidationError(message: string) {
  validationError = message;
  const binding = message.match(/\$\.bindings\[(\d+)\]/),
    animation = message.match(/\$\.animations\.([a-zA-Z0-9_-]+)/),
    idleGroup = message.match(/\$\.idleGroups\[(\d+)\]/);
  if (binding) {
    tab = "events";
    selectedBinding = Number(binding[1]);
  } else if (animation) {
    tab = "animations";
    selectedClip = animation[1];
  } else if (message.includes("$.idle") || message.includes("$.defaultAnimation")) {
    tab = "idle";
    selectedIdleGroup = idleGroup ? doc?.project.idleGroups?.[Number(idleGroup[1])]?.id ?? "" : "";
  }
  else if (message.includes("$.cursorFollow")) tab = "follow";
  else if (message.includes("$.defaults.timing")) tab = "timing";
  else if (message.includes("$.defaults.displayHeightDip")) tab = "overview";
  else if (message.includes("$.build.macosDmg")) tab = "builds";
  else if (message.includes("$.product")) tab = "overview";
  else if (message.includes("project decoded images exceed")) tab = "resources";
  layout();
  requestAnimationFrame(() =>
    document.querySelector<HTMLElement>("#validation-panel")?.focus(),
  );
}
function pathValue(path: string) {
  return path.split(".").reduce((o, k) => o?.[k], doc as any);
}
function setPath(path: string, value: unknown) {
  const keys = path.split(".");
  let o: any = doc;
  for (const key of keys.slice(0, -1)) o = o[key];
  o[keys.at(-1)!] = value;
  markDirty();
}
const field = (label: string, path: string, type = "text", extra = "") =>
  `<label class="field"><span>${label}</span><input type="${type}" data-path="${path}" value="${esc(pathValue(path))}" ${extra}></label>`;
const check = (label: string, path: string) =>
  `<label class="toggle"><input type="checkbox" data-path="${path}" ${pathValue(path) ? "checked" : ""}><span>${label}</span></label>`;
const option = (value: string, label: string, current: string) =>
  `<option value="${esc(value)}" ${value === current ? "selected" : ""}>${esc(label)}</option>`;
const clips = () => Object.keys(doc!.character.animations);
const resourceCount = () => Object.keys(doc!.character.resources).length
  + Object.keys(doc!.character.sounds).length + (doc!.character.image ? 1 : 0);
const payload = () => ({
  revision: doc!.meta.revision,
  draftRevision,
  project: doc!.project,
  character: doc!.character,
  license: doc!.license,
});
function stopPreview() {
  simulationRequest++;
  renderer?.dispose();
  renderer = undefined;
  previewDocument = undefined;
  previewClip = "";
  setPreviewWork();
  simulationTimers.forEach(clearTimeout);
  simulationTimers = [];
  previewAudio.stop();
}
function layout(push = false) {
  clearTimeout(gifFrameTimer);
  gifFrameRequest++;
  for (const preview of scalePreviews) preview.dispose();
  scalePreviews = [];
  const canvas = doc && previewDocument === saved && renderer
    ? document.querySelector<HTMLCanvasElement>("#preview-canvas")
    : null;
  const previewState = canvas ? document.querySelector("#preview-state")?.textContent : null;
  if (!canvas) stopPreview();
  if (doc && !doc.character.animations[selectedClip]) selectedClip = doc.character.defaultAnimation;
  app.innerHTML = `<aside class="sidebar"><a class="brand" href="/"><span class="brand-mark">b<span>•</span></span><span>buddy<span class="brand-studio">STUDIO</span></span></a><div class="workspace-label">LOCAL WORKSPACE <span class="local-dot"></span></div><div class="sidebar-heading"><h2>프로젝트 <span>${projects.filter((p) => p.archived === archived).length}</span></h2><button class="icon-button" data-action="new" aria-label="프로젝트 만들기">＋</button></div><div class="project-list">${
    projects
      .filter((p) => p.archived === archived)
      .map(
        (p, i) =>
          `<button class="project ${p.id === doc?.meta.id ? "selected" : ""}" data-action="open" data-id="${p.id}"><span class="project-avatar tone-${i % 4}">${esc(p.name[0])}</span><span><strong>${esc(p.name)}</strong><small title="${esc(p.id)}">${esc(p.id)}</small></span><span class="project-dot">${p.archived ? "◌" : "↗"}</span></button>`,
      )
      .join("") ||
    '<p class="empty-small">아직 프로젝트가 없습니다.<br>새로운 버디를 만들어보세요.</p>'
  }</div><button class="sidebar-new" data-action="new"><span>＋</span> 새 프로젝트</button><div class="sidebar-bottom"><button data-action="archive-list">${archived ? "← 작업 중인 프로젝트" : "▤ 보관한 프로젝트"}</button><span class="workspace-path" title="${esc(config.dataRoot)}">프로젝트는 이 컴퓨터에 저장됩니다.</span><span class="version">DESKTOP BUDDY STUDIO <span>v0.1</span></span></div></aside><div class="workspace">${doc ? editor() : welcome()}</div><div id="toast" class="toast" role="status"></div><dialog id="project-dialog"><form id="new-project-form"><span class="eyebrow">A NEW COMPANION</span><h2>새 버디 프로젝트</h2><p>이름과 고유한 패키지 ID를 정해주세요.</p><label class="field"><span>프로젝트 · 앱 이름</span><input name="name" required maxlength="80" value="My Buddy" autofocus></label><label class="field"><span>패키지 ID · 프로젝트 폴더와 URL</span><input name="packageId" required maxlength="128" value="io.desktopbuddy.app.mybuddy" placeholder="com.example.mybuddy" spellcheck="false" autocomplete="off"><small>예: com.example.typingcat · 생성 후 고정됩니다.</small></label><label class="field"><span>시작 템플릿</span><select name="template"><option value="empty">빈 프로젝트 · 직접 리소스 가져오기</option><option value="typing-cat">Typing Cat · 키보드에 반응하는 고양이</option><option value="click-mascot">Click Mascot · 마우스에 반응하는 버디</option></select></label><p id="new-project-error" class="error-text" role="alert" hidden></p><footer><button type="button" data-action="close-dialog">취소</button><button class="primary" type="submit">프로젝트 만들기 ↗</button></footer></form></dialog>`;
  updateEditingStatus();
  setConnection(connectionIssue);
  if (projectLoading) setProjectLoading(true);
  if (doc) {
    syncRoute(push);
    if (canvas) {
      document.querySelector("#preview-canvas")!.replaceWith(canvas);
      document.querySelector("#preview-state")!.textContent = previewState ?? selectedClip;
      if (previewClip !== selectedClip) playClip(selectedClip);
    } else startPreview();
    setPreviewWork(previewWork);
    if (tab === "animations") {
      startScaleComparison();
      setupCrop();
      const input = document.querySelector<HTMLInputElement>("#gif-frame-number");
      if (input) showGifFrame(input.valueAsNumber, 0);
    }
    if (doc.meta.archived) {
      for (const input of document.querySelectorAll<
        HTMLInputElement | HTMLSelectElement | HTMLTextAreaElement
      >(
        ".editor-content input,.editor-content select,.editor-content textarea",
      ))
        input.disabled = true;
      for (const button of document.querySelectorAll<HTMLButtonElement>(
        ".editor-content button[data-action]",
      ))
        if (!["load-revisions", "clip", "binding", "reference-clip", "reference-binding"].includes(button.dataset.action ?? "")) button.disabled = true;
      const saveButton = document.querySelector<HTMLButtonElement>(
        'button[data-action="save"]',
      );
      if (saveButton) saveButton.disabled = true;
    }
  }
}
function welcome() {
  return `<div class="welcome"><span class="eyebrow">YOUR DESKTOP, A LITTLE MORE ALIVE</span><h1>나만의 데스크톱 버디 만들기</h1><p>이미지나 GIF를 가져와 키보드와 마우스에 반응하는 버디를 만드세요.</p><button class="primary" data-action="new">＋ 첫 프로젝트 만들기</button><div class="welcome-cards"><article><span>01</span><h3>리소스 가져오기</h3><p>PNG, GIF, JPG, WebP, BMP</p></article><article><span>02</span><h3>반응 정의하기</h3><p>키보드 · 입력 속도 · 키워드 · 드래그</p></article><article><span>03</span><h3>앱으로 빌드하기</h3><p>프로젝트마다 독립된 앱과 패키지</p></article></div></div>`;
}
function editor() {
  return `<header class="topbar"><div class="breadcrumb">워크스페이스 <span>/</span> ${esc(doc!.project.product.name)}</div><div class="top-actions"><span id="save-state" role="status" aria-live="polite" class="save-state ${dirty ? "dirty" : ""}">${dirty ? "저장하지 않은 변경" : "모든 변경 저장됨"}</span><button id="cancel-edit" class="quiet" data-action="discard-draft" hidden>변경 취소</button><button data-action="validate">검증</button><button data-action="save" title="Ctrl / ⌘ + S">저장</button><button class="primary" data-action="tab" data-id="builds" ${doc!.meta.archived ? "disabled" : ""}>앱 빌드 <span>↗</span></button></div></header><div class="project-heading"><div><div class="eyebrow">BUDDY PROJECT <span class="pill">${doc!.meta.archived ? "보관됨" : "작업 중"}</span></div><h1>${esc(doc!.project.product.name)}</h1></div><div class="project-tools"><button class="quiet" data-action="clone">복제</button><button class="quiet" data-action="archive">${doc!.meta.archived ? "복원" : "보관"}</button></div></div><div class="editor-grid"><main class="editor"><nav class="tabs" aria-label="프로젝트 편집">${[
    ["overview", "프로젝트"],
    ["resources", "리소스"],
    ["animations", "애니메이션"],
    ["idle", "대기 상태"],
    ["follow", "마우스 따라오기"],
    ["events", "이벤트"],
    ["timing", "반응 타이밍"],
    ["builds", "빌드"],
  ]
    .map(
      ([id, label]) =>
        `<button data-action="tab" data-id="${id}" class="${tab === id ? "active" : ""}">${label}${id === "resources" ? ` <span>${resourceCount()}</span>` : ""}</button>`,
    )
    .join(
      "",
    )}</nav><div class="editor-content">${validationPanel()}${restoredDraft && dirty ? '<div class="draft-banner"><div><strong>임시 편집을 복원했습니다.</strong><p>저장하면 미리보기와 다음 빌드에 반영됩니다.</p></div><button class="small" data-action="discard-draft">편집 버리기</button></div>' : ""}${({ overview: overview, resources: resources, animations: animations, idle: idleSettings, follow: followSettings, events: events, timing: timing, builds: builds }[tab] ?? overview)()}</div></main>${previewPanel()}</div><footer class="workspace-footer"><span><i class="local-dot"></i> 로컬 프로젝트 · ${esc(doc!.project.product.version)}</span><span title="${esc(doc!.contentHash)}">${esc(doc!.contentHash.slice(0, 12))} · ${date(doc!.meta.updatedAt)}</span></footer>`;
}
function title(kicker: string, name: string, description: string) {
  return `<div class="section-title"><span class="eyebrow">${kicker}</span><h2>${name}</h2><p>${description}</p></div>`;
}
function firstSpriteCard() {
  return '<div class="card"><h3>첫 스프라이트를 추가하세요</h3><p class="hint">리소스에서 이미지나 GIF를 가져오면 기본 대기 동작을 만들 수 있습니다.</p><button class="small" data-action="tab" data-id="resources">리소스로 이동 →</button></div>';
}
function defaultSizeFields() {
  const defaults = doc!.project.defaults;
  const absolute = defaults.displayHeightDip != null;
  return `<label class="field"><span>기본 크기 기준</span><select id="display-size-mode">${option("height", "대기 애니메이션의 표시 높이 (DIP)", absolute ? "height" : "scale")}${option("scale", "리소스 기준 배율", absolute ? "height" : "scale")}</select></label>${absolute
    ? field("기본 대기 높이 (DIP)", "project.defaults.displayHeightDip", "number", 'min="16" max="1024" step="any"')
    : field("기본 크기 배율", "project.defaults.scale", "number", 'min="0.25" max="3" step="0.05"')}`;
}
function setTimerEnabled(enabled: boolean) {
  if (!doc || doc.meta.archived) return;
  doc.project.timer ??= { enabled: false, mode: "countdown", countdownSeconds: 600,
    pomodoro: { focusMinutes: 25, shortBreakMinutes: 5, longBreakMinutes: 15, rounds: 4 } };
  doc.project.timer.enabled = enabled;
  markDirty();
  layout();
}
function timerDurationFields(seconds: number, attribute: "data-timer-time" | "data-timer-preset-unit") {
  const fields = [
    ["시간", Math.floor(seconds / 3600), 3],
    ["분", Math.floor(seconds / 60) % 60, 59],
    ["초", seconds % 60, 59],
  ] as const;
  return `<div class="form-grid timer-duration-fields">${fields.map(([label, value, max]) =>
    `<label class="field"><span>${label}</span><input ${attribute} type="number" inputmode="numeric" min="0" max="${max}" step="1" value="${value}"></label>`).join("")}</div>`;
}
function readTimerDuration(fields: HTMLInputElement[]) {
  const seconds = fields.reduce((sum, field, index) => sum + Number(field.value) * [3600, 60, 1][index], 0);
  return fields.every(field => field.validity.valid) && seconds >= 1 && seconds <= 10800 ? seconds : 0;
}
function timerPresetWarning(presets: readonly number[]) {
  if (presets.some(seconds => !seconds)) return "바로가기 시간은 1초부터 3시간 사이로 입력하세요.";
  return new Set(presets).size === presets.length ? "" : "같은 시간의 바로가기는 하나만 등록할 수 있습니다.";
}
function timerSettings() {
  const timer = doc!.project.timer;
  const toggle = `<label class="toggle"><input type="checkbox" id="timer-enabled" ${timer?.enabled ? "checked" : ""}><span>타이머 사용</span></label>`;
  if (!timer?.enabled) return `<div class="card"><h3>타이머</h3>${toggle}</div>`;
  const presets = timer.countdownPresets ?? defaultTimerPresets;
  return `<div class="card"><h3>타이머</h3>${toggle}
    <label class="field"><span>처음 열 모드</span><select data-path="project.timer.mode">${option("countdown", "일반 타이머", timer.mode)}${option("pomodoro", "뽀모도로", timer.mode)}</select></label>
    <fieldset><legend>일반 타이머 기본 시간</legend>${timerDurationFields(timer.countdownSeconds, "data-timer-time")}
    <p class="hint" id="timer-time-hint">처음 직접 입력할 때 채워지는 값입니다. 1초부터 3시간까지 설정할 수 있습니다.</p></fieldset>
    <div class="card-heading"><h3>일반 타이머 바로가기</h3><button type="button" class="small" data-action="timer-preset-add" ${presets.length >= timerPresetLimit ? "disabled" : ""}>＋ 바로가기</button></div>
    <p class="hint">Player에 배포할 바로가기입니다. 표시 순서대로 누르면 바로 시작하며, Player에서 개인 목록을 저장하거나 이 목록으로 복원할 수 있습니다.</p>
    ${presets.map((seconds, index) => `<fieldset data-timer-preset="${index}"><legend>바로가기 ${index + 1}</legend>
      ${timerDurationFields(seconds, "data-timer-preset-unit")}
      <div class="row-actions">
        <button type="button" class="small" data-action="timer-preset-move" data-id="${index}:-1" ${index === 0 ? "disabled" : ""}>위로</button>
        <button type="button" class="small" data-action="timer-preset-move" data-id="${index}:1" ${index === presets.length - 1 ? "disabled" : ""}>아래로</button>
        <button type="button" class="text-button danger" data-action="timer-preset-remove" data-id="${index}">삭제</button>
      </div></fieldset>`).join("")}
    <p class="error-text" id="timer-preset-warning">${timerPresetWarning(presets)}</p>
    <div class="form-grid">
      ${field("집중 (분)", "project.timer.pomodoro.focusMinutes", "number", 'min="1" max="120" step="1"')}
      ${field("짧은 휴식 (분)", "project.timer.pomodoro.shortBreakMinutes", "number", 'min="1" max="60" step="1"')}
      ${field("긴 휴식 (분)", "project.timer.pomodoro.longBreakMinutes", "number", 'min="1" max="60" step="1"')}
      ${field("긴 휴식까지 집중 완료 (회)", "project.timer.pomodoro.rounds", "number", 'min="2" max="8" step="1"')}
    </div><p class="hint">Player에서 자주 쓰는 시간과 개인 뽀모도로 설정을 저장할 수 있습니다.</p>
    <button class="small" data-action="tab" data-id="events">완료 반응 설정 →</button></div>`;
}
function overview() {
  return `${title("PROJECT DETAILS", "프로젝트 설정", "앱 이름, 표시 크기와 기본 동작을 설정합니다.")}<div class="card"><div class="form-grid">${field("앱 이름", "project.product.name")}${field("실행 파일 이름 (slug)", "project.product.slug", "text", (doc!.meta.builtSlug || jobs.some((j) => (!j.productIdentifier || j.productIdentifier === doc!.meta.identifier) && (j.status === "succeeded" || j.status === "partial"))) ? "readonly" : "")}${field("버전", "project.product.version")}${field("캐릭터 이름", "character.name")}</div><label class="field"><span>패키지 ID <small>프로젝트 폴더·URL·앱 식별자로 사용합니다.</small></span><input value="${esc(doc!.project.product.identifier)}" readonly></label></div><div class="card"><h3>데스크톱에서의 모습</h3><div class="form-grid">${defaultSizeFields()}${field("처음 배치 여백 (DIP)", "project.defaults.position.marginDip", "number", 'min="0" max="256"')}${field("기준 프레임 너비 (px)", "character.sheet.frameWidth", "number", 'min="16" max="512"')}${field("기준 프레임 높이 (px)", "character.sheet.frameHeight", "number", 'min="16" max="512"')}</div><p class="hint">표시 높이는 투명 여백을 제외한 기본 대기 애니메이션의 높이입니다. 화면에 맞춰 줄어들 수 있으며, 화면 가장자리에서는 모든 동작에 여백을 둡니다.</p>${check("항상 위에 표시", "project.defaults.alwaysOnTop")}${check("키보드 감지", "project.defaults.input.keyboard")}${check("마우스 감지", "project.defaults.input.mouse")}${check("키를 누른 채 이동", "project.defaults.movementEnabled")}</div>${timerSettings()}<div class="card"><h3>앱 아이콘 · 리소스 이용 조건</h3><div class="icon-upload"><img src="/api/projects/${doc!.meta.id}/source/assets/app-icon.png" alt="앱 아이콘"><div><strong>앱 아이콘</strong><p>512~1024px 정사각형 PNG</p><label class="button small">아이콘 교체<input type="file" id="icon-upload" accept="image/png" hidden></label></div></div><label class="field"><span>프로젝트 리소스 라이선스</span><textarea data-path="license" rows="5">${esc(doc!.license)}</textarea></label><p class="hint">가져온 리소스의 이용 조건과 출처를 여기에 모아두세요. 빌드 패키지에 함께 포함됩니다.</p></div>`;
}
function resourceReferences(id: string) {
  const used = Object.entries(doc!.character.animations).filter(([, a]) => (a.resource ?? "sheet") === id);
  return `<div class="usage-links"><span>사용 동작 ${used.length}</span>${used.map(([name]) => `<button class="chip" data-action="reference-clip" data-id="${esc(name)}">${esc(name)} →</button>`).join("") || '<small>아직 연결된 동작이 없습니다.</small>'}</div>`;
}
function usesAnimation(effect: Effect, name: string) {
  return effect.type === "faceCursor"
    ? Object.values(effect.directions ?? {}).some((pose) => pose.animation === name)
    : effect.animation === name || (effect.type === "playAnimation" && effect.variation?.alternatives.includes(name));
}
function usedByIdle(name: string) {
  return doc!.character.defaultAnimation === name
    || Object.values(doc!.project.idle?.directions ?? {}).some((pose) => pose.animation === name)
    || [...(doc!.project.idleGroups ?? []), ...(idleGroupDraft ? [idleGroupDraft] : [])].some(group =>
      group.animation === name || Object.values(group.directions ?? {}).some(pose => pose.animation === name));
}
function animationReferences(name: string) {
  const used = doc!.project.bindings.map((b, i) => ({ b, i })).filter(({ b }) => usesAnimation(b.effect, name));
  return `<div class="usage-links"><span>연결 이벤트 ${used.length}</span>${used.map(({ b, i }) => `<button class="chip" data-action="reference-binding" data-id="${i}">${esc(b.id)} →</button>`).join("") || "<small>아직 연결된 이벤트가 없습니다.</small>"}${usedByIdle(name) ? '<button class="chip" data-action="tab" data-id="idle">대기 상태 →</button>' : ""}${doc!.project.cursorFollow?.bands.some(band => band.animation === name) ? '<button class="chip" data-action="tab" data-id="follow">마우스 따라오기 →</button>' : ""}</div>`;
}
const isAudio = (filename: string) => /\.(mp3|wav)$/i.test(filename);
const isImage = (filename: string) => /\.(png|jpe?g|gif|webp|bmp)$/i.test(filename);
function originalResourceName(id: string, resource: Resource | SoundResource) {
  const path = "image" in resource ? resource.image : resource.audio;
  return doc!.library[path.slice("resources/".length)]?.originalName ?? id;
}
function resourceName(id: string, resource: Resource | SoundResource) {
  return resource.name ?? originalResourceName(id, resource);
}
function soundCards() {
  const sounds = Object.entries(doc!.character.sounds);
  if (!sounds.length) return "";
  return `<div class="card"><h3>사운드 <span class="count">${sounds.length}</span></h3>${sounds.map(([id, sound]) => {
    const file = sound.audio.slice(10);
    const name = resourceName(id, sound);
    const bindings = doc!.project.bindings.map((binding, index) => ({ binding, index }))
      .filter(({ binding }) => binding.sound?.resource === id);
    return `<div class="sound-resource"><div class="card-heading"><strong title="${esc(originalResourceName(id, sound))}">${esc(name)}</strong><div class="row-actions"><button class="text-button" data-action="rename-sound" data-id="${esc(id)}" aria-label="${esc(name)} 이름 수정">이름 수정</button><button class="text-button danger" data-action="remove-sound" data-id="${esc(id)}">제외</button></div></div><audio controls preload="none" aria-label="${esc(name)} 들어보기" src="${resourceUrl(file)}"></audio><div class="usage-links">${bindings.map(({ binding, index }) => `<button class="chip" data-action="reference-binding" data-id="${index}">${esc(binding.id)} →</button>`).join("")}</div></div>`;
  }).join("")}</div>`;
}
function resources() {
  const entries = Object.entries(doc!.character.resources ?? {});
  const library = Object.values(doc!.library).filter(file => isImage(file.name) || isAudio(file.name));
  const image = doc!.character.image;
  const sheetInUse = Object.values(doc!.character.animations).some(animation => !animation.resource);
  const baseSprite = image ? `<article class="resource-card"><div class="resource-art checker"><img src="${sourceUrl(image)}" alt="기본 스프라이트"></div><div><strong>기본 스프라이트</strong><span>PNG · ${doc!.character.sheet.width} × ${doc!.character.sheet.height}</span></div><div class="resource-bottom"><small class="resource-id">sheet · 기본 제공</small><button class="text-button danger" data-action="remove-base-sprite" ${sheetInUse ? 'disabled title="이 스프라이트를 사용하는 동작을 먼저 변경하세요."' : ""}>제외</button></div>${resourceReferences("sheet")}</article>` : "";
  return `${title("RESOURCE LIBRARY", "이미지와 사운드", "이미지와 사운드를 가져와 애니메이션이나 이벤트에 연결하세요.")}<label class="dropzone" id="dropzone"><input type="file" id="resource-upload" accept=".png,.jpg,.jpeg,.gif,.webp,.bmp,.wav,.mp3" multiple hidden><span class="upload-symbol">↥</span><strong>${doc!.character.defaultAnimation ? "이미지나 사운드를 놓거나 클릭해서 가져오기" : "첫 스프라이트를 놓거나 클릭해서 가져오기"}</strong><span>PNG · JPG · GIF · WebP · BMP · WAV · MP3 / 파일당 최대 16 MiB</span></label><p class="hint">이미지와 GIF는 표시할 때 기준 프레임에 자동으로 맞춥니다. <button class="text-button" data-action="tab" data-id="overview">크기 기준 ${doc!.character.sheet.frameWidth} × ${doc!.character.sheet.frameHeight} px</button></p><div class="resource-grid">${baseSprite}${entries
    .map(([id, r]) => {
      const f = r.image.slice(10),
        file = doc!.library[f],
        name = resourceName(id, r);
      return `<article class="resource-card"><div class="resource-art checker"><img src="${resourceUrl(f)}" alt="${esc(name)}" loading="lazy" decoding="async"></div><div><strong title="${esc(originalResourceName(id, r))}">${esc(name)}</strong><span>${esc(f.split(".").at(-1)?.toUpperCase())} · ${file ? prettySize(file.bytes) : ""}</span></div><div class="resource-bottom"><small class="resource-id">${esc(id)}</small><div class="row-actions"><button class="text-button" data-action="rename-resource" data-id="${esc(id)}" aria-label="${esc(name)} 이름 수정">이름 수정</button><button class="text-button danger" data-action="remove-resource" data-id="${esc(id)}">제외</button></div></div>${resourceReferences(id)}</article>`;
    })
    .join(
      "",
    )}</div>${soundCards()}<p class="hint">GIF는 프레임과 재생 시간을 읽어 이벤트마다 처음부터 재생합니다. 애니메이션 WebP/APNG는 GIF 또는 스프라이트로 변환해서 가져오세요.</p><div class="card"><h3>보관함 <span class="count">${library.length}</span></h3><p class="hint">구성에서 제외해도 원본은 남아 있습니다. 이전 버전 복원과 재사용에 사용할 수 있습니다.</p>${
    library
      .map((f) => {
        const used = isAudio(f.name)
          ? Object.values(doc!.character.sounds).find((sound) => sound.audio === `resources/${f.name}`)
          : entries.find(([, resource]) => resource.image === `resources/${f.name}`)?.[1];
        return `<div class="library-row"><span title="${esc(f.originalName)}">${esc(used?.name ?? f.originalName)} <small>${prettySize(f.bytes)}</small></span><button class="text-button" data-action="reuse-resource" data-id="${f.name}" ${used ? "disabled" : ""}>${used ? "사용 중" : "구성에 추가"}</button></div>`;
      })
      .join("") || '<p class="hint">아직 가져온 원본 파일이 없습니다.</p>'
  }</div>`;
}
function animations() {
  const names = clips();
  if (!names.length) return firstSpriteCard();
  if (!names.includes(selectedClip)) selectedClip = names[0];
  const a = doc!.character.animations[selectedClip],
    p = `character.animations.${selectedClip}`;
  const isGif =
    !!a.resource &&
    doc!.character.resources[a.resource]?.image.toLowerCase().endsWith(".gif");
  return `${title("ANIMATION CLIPS", "애니메이션 편집", "한 장의 이미지, GIF 전체 또는 스프라이트의 특정 영역을 하나의 동작으로 정의합니다.")}<div class="subheading"><div class="clip-chips">${names.map((n) => `<button class="chip ${n === selectedClip ? "active" : ""}" data-action="clip" data-id="${esc(n)}">${esc(n)} <small>${doc!.character.animations[n].loop ? "↻" : "→"}</small></button>`).join("")}</div><button class="small" data-action="add-clip">＋ 동작</button></div><div class="card"><div class="card-heading"><h3>${esc(selectedClip)}</h3>${selectedClip !== doc!.character.defaultAnimation ? '<button class="text-button danger" data-action="remove-clip">동작 삭제</button>' : '<span class="pill">기본 대기 동작</span>'}</div><label class="field"><span>표시할 리소스</span><select id="clip-resource">${doc!.character.image ? option("sheet", "기본 PNG 스프라이트", a.resource ?? "sheet") : ""}${Object.entries(
    doc!.character.resources ?? {},
  )
    .map(([id, r]) =>
      option(
        id,
        `${resourceName(id, r)} · ${id}`,
        a.resource ?? "sheet",
      ),
    )
    .join(
      "",
    )}</select></label><div class="form-grid">${field("재생 속도 (FPS)", `${p}.fps`, "number", 'min="1" max="30"')}${check("반복 재생", `${p}.loop`)}</div>${isGif ? check("GIF 원본 프레임 시간 사용", `${p}.useSourceTiming`) : a.resource ? "" : `<label class="field"><span>스프라이트 프레임 번호 (0부터, 쉼표로 구분)</span><input id="clip-frames" value="${a.frames.join(", ")}" placeholder="0, 1, 2, 3"></label>`}<p class="hint">일회성 이벤트는 반복을 끄고, 입력 속도·키 유지·드래그 동작은 반복을 켜세요. GIF는 프레임과 영역을 비워두면 전체를 사용합니다.</p></div>${scaleEditor(a)}${animationReferences(selectedClip)}${isGif ? gifFramesEditor(a) : cropEditor(a)}`;
}
function anchorPercent(anchor: number | null | undefined) {
  return anchor == null ? "" : String(Number((anchor * 100).toFixed(8)));
}
function scaleEditor(animation: Animation) {
  const size = animation.scalePercent ?? 100;
  const idle = doc!.character.defaultAnimation;
  const limits = `min="${animationScaleLimits.min}" max="${animationScaleLimits.max}" step="1"`;
  const canCompare = !!saved?.renderPlan.animations[idle] && !!saved.renderPlan.animations[selectedClip];
  return `<div class="card"><div class="card-heading"><h3>동작 크기와 방향 맞추기</h3><button class="small" data-action="reset-clip-scale" id="reset-clip-scale" ${size === 100 ? "disabled" : ""}>100%로 초기화</button></div>
    <div class="clip-scale-controls">
      <label class="field"><span>표시 배율</span><input id="clip-scale-range" type="range" ${limits} value="${size}"></label>
      <label class="field"><span>배율 (%)</span><input id="clip-scale-number" type="number" ${limits} value="${size}"></label>
    </div>
    <p class="hint">100%는 자동 맞춤 크기입니다. 배율은 동작의 모든 프레임에 같이 적용됩니다.</p>
    <div class="form-grid">
      <label class="field"><span>원본이 보는 방향</span><select id="clip-source-facing">${option("right", "오른쪽", animation.sourceFacing ?? "right")}${option("left", "왼쪽", animation.sourceFacing ?? "right")}</select></label>
      <label class="field"><span>가로 기준점 (%)</span><input id="clip-anchor-x" type="number" min="0" max="100" step="any" placeholder="공통값 ${anchorPercent(doc!.character.anchor.x)}%" value="${anchorPercent(animation.anchorX)}"></label>
    </div>
    <p class="hint">가로 기준점은 프레임 왼쪽에서 잰 위치입니다. 비워 두면 캐릭터 공통 기준점을 사용합니다.</p>
    ${canCompare ? `<label class="field"><span>선택한 동작 미리보기 방향</span><select id="clip-preview-facing"><option value="">원본</option><option value="left">왼쪽 보기</option><option value="right">오른쪽 보기</option></select></label>
    <div class="clip-scale-compare">
      <figure><div class="clip-scale-stage checker"><canvas id="clip-scale-reference" aria-label="기준 대기 동작"></canvas></div><figcaption>${idle === selectedClip ? "저장한 대기" : "기준 대기"} · ${esc(idle)}</figcaption></figure>
      <figure><div class="clip-scale-stage checker"><canvas id="clip-scale-preview" aria-label="선택한 동작의 크기"></canvas></div><figcaption>선택한 동작 · ${esc(selectedClip)}</figcaption></figure>
    </div><p class="hint">중앙선은 방향을 바꿔도 움직이지 않는 기준점입니다.</p>` : '<p class="hint">동작을 한 번 저장하면 대기 동작과 나란히 비교할 수 있습니다.</p>'}
  </div>`;
}
function gifFrameUrl(file: string, index: number) {
  return `/api/projects/${doc!.meta.id}/gif-frame/${encodeURIComponent(file)}/${index}`;
}
function showGifFrame(index: number, delay = 160) {
  clearTimeout(gifFrameTimer);
  const request = ++gifFrameRequest;
  const animation = doc!.character.animations[selectedClip];
  const filename = doc!.character.resources[animation.resource!].image.slice(10);
  const status = document.querySelector<HTMLElement>("#gif-frame-status")!;
  status.textContent = `${index}번 프레임을 불러오는 중…`;
  gifFrameTimer = window.setTimeout(() => {
    const image = new Image();
    image.id = "gif-frame-preview";
    image.alt = `GIF ${index}번 프레임`;
    image.onload = () => {
      if (request !== gifFrameRequest) return;
      document.querySelector("#gif-frame-preview")!.replaceWith(image);
      status.textContent = `${index}번 프레임`;
    };
    image.onerror = () => {
      if (request === gifFrameRequest)
        status.textContent = `${index}번 프레임을 불러오지 못했습니다.`;
    };
    image.src = gifFrameUrl(filename, index);
  }, delay);
}
function gifFramesEditor(animation: Animation) {
  const filename = doc!.character.resources[animation.resource!].image.slice(10);
  const count = doc!.library[filename]?.frames;
  const first = animation.frames[0] ?? 0;
  return `<div class="card"><h3>GIF 프레임 선택</h3><p class="hint">재생 목록을 비우면 원본 전체를 재생합니다. 번호 순서대로 재생하며 같은 프레임을 반복해서 넣을 수 있습니다.</p>
    <div class="gif-frame-preview checker"><img id="gif-frame-preview" alt="" hidden></div>
    <p id="gif-frame-status" class="hint" role="status"></p>
    <div class="row-actions"><label class="field"><span>확인할 프레임 번호${count ? ` (0~${count - 1})` : ""}</span><input id="gif-frame-number" type="number" min="0" max="${(count ?? 256) - 1}" value="${first}" step="1"></label>
    <button class="small" data-action="append-gif-frame">＋ 재생 목록에 추가</button><button class="small" data-action="clear-gif-frames">전체 프레임 사용</button></div>
    <label class="field"><span>재생 목록 (쉼표로 구분)</span><input id="clip-frames" value="${animation.frames.join(", ")}" placeholder="전체 프레임"></label></div>`;
}
function cropEditor(a: Animation) {
  return `<div class="card"><div class="card-heading"><h3>영역 선택 <span class="pill">SPRITE CROP</span></h3><button class="text-button" data-action="clear-regions">전체 이미지로 초기화</button></div><p class="hint">이미지 위를 드래그해 표시할 사각형을 선택하세요. GIF에는 영역 자르기를 적용할 수 없습니다.</p><div class="crop-viewport checker"><canvas id="crop-canvas" aria-label="스프라이트 영역 선택"></canvas></div><div class="crop-fields">${["x", "y", "width", "height"].map((key, i) => `<label class="field"><span>${["X", "Y", "너비", "높이"][i]}</span><input type="number" min="${i < 2 ? 0 : 1}" id="crop-${key}" value="${crop[key as keyof FrameRegion]}"></label>`).join("")}</div><div class="row-actions"><button class="small" data-action="apply-region">이 영역만 표시</button><button class="small" data-action="append-region">＋ 다음 프레임으로 추가</button></div><div class="region-list">${(a.regions ?? []).map((r, i) => `<span class="region">${i}: ${r.x},${r.y} · ${r.width}×${r.height}<button data-action="remove-region" data-id="${i}" aria-label="${i}번 영역 제외">×</button></span>`).join("")}</div><details><summary>격자로 프레임 만들기</summary><div class="crop-fields">${[
    ["grid-width", "셀 너비", doc!.character.sheet.frameWidth],
    ["grid-height", "셀 높이", doc!.character.sheet.frameHeight],
    ["grid-start", "시작 번호", 0],
    ["grid-count", "프레임 수", 4],
  ]
    .map(
      ([id, label, value]) =>
        `<label class="field"><span>${label}</span><input id="${id}" type="number" min="${String(id).includes("start") ? 0 : 1}" value="${value}"></label>`,
    )
    .join(
      "",
    )}</div><button class="small" data-action="make-grid">격자 영역 적용</button></details></div>`;
}
const triggerNames: Record<string, string> = {
  keyDown: "키 누름",
  keyUp: "키 뗌",
  whileKeyDown: "키 유지",
  typingRate: "입력 속도",
  keySequence: "키워드 입력",
  mouseInput: "마우스·트랙패드",
  drag: "버디 드래그",
  dragDropped: "드래그 후 놓기",
  firstShown: "첫 표시",
  shownAgain: "다시 표시",
  idleFor: "버디 대기 상태 지속",
  typingStopped: "타이핑 후 멈춤",
  buddyLongPress: "버디 길게 누르기",
  timerStarted: "타이머 시작",
  timerCompleted: "타이머 완료",
};
const timerPhaseLabels: Record<TimerPhase, string> = {
  countdown: "일반 타이머", focus: "집중", shortBreak: "짧은 휴식", longBreak: "긴 휴식",
};
const timerPhases = Object.keys(timerPhaseLabels) as TimerPhase[];
function automaticTrigger(type: string) {
  return type === "firstShown" || type === "shownAgain" || type === "idleFor" || type === "typingStopped" || type === "timerCompleted" || type === "timerStarted";
}
function triggerCategory(trigger: Binding["trigger"]) {
  return pointerInputs(trigger) ? "mouseInput" : trigger.type;
}
function events() {
  if (!doc!.character.defaultAnimation) return firstSpriteCard();
  const bindings = doc!.project.bindings;
  if (selectedBinding >= bindings.length)
    selectedBinding = Math.max(0, bindings.length - 1);
  const b = bindings[selectedBinding];
  return `${title("EVENT MAPPINGS", "이벤트 편집", "입력, 버디 조작, 표시와 대기 상태에 연결할 동작을 설정합니다.")}<div class="subheading"><span class="hint">${bindings.length}개의 이벤트 · 우선순위가 높을수록 먼저 표시됩니다. 같으면 보조 키 조건을 지정한 이벤트, 그다음 목록에서 위에 있는 이벤트가 우선합니다. 드래그 중에는 드래그 동작이 먼저 표시됩니다. 자동 동작보다 입력과 이동이 우선합니다.</span><div class="row-actions"><button class="small" data-action="tab" data-id="idle">대기 상태 설정</button><button class="small" data-action="add-binding">＋ 이벤트</button></div></div><div class="binding-list">${bindings.map((b, i) => `<button class="binding-row ${i === selectedBinding ? "active" : ""}" data-action="binding" data-id="${i}"><span class="binding-icon">${b.trigger.type === "typingRate" ? "↯" : b.trigger.type === "drag" ? "✥" : "⌘"}</span><span><strong>${esc(b.id)}</strong><small>${triggerNames[triggerCategory(b.trigger)]} ${esc(b.trigger.type === "idleFor" ? `${b.trigger.seconds}초` : inputChoices(b.trigger).filter(Boolean).join(" / "))}</small></span><span class="binding-arrow">→</span><span class="binding-animation">${esc(b.effect.type === "faceCursor" ? "방향 전환" : b.effect.type === "playAnimation" && b.effect.variation ? `${1 + b.effect.variation.alternatives.length}개 모션` : b.effect.animation ?? "방향 전환")}</span></button>`).join("")}</div>${b ? bindingEditor(b) : '<div class="card empty-small">이벤트를 추가해서 동작과 연결하세요.</div>'}`;
}
function followSettings() {
  const heading = title("CURSOR FOLLOW", "마우스 따라오기", "마우스까지의 거리에 따라 이동 속도와 애니메이션을 바꿉니다.");
  const config = doc!.project.cursorFollow;
  if (!config) return `${heading}<div class="card"><button class="small" data-action="configure-follow">따라오기 설정</button></div>`;
  const path = "project.cursorFollow";
  const looping = clips().filter(name => doc!.character.animations[name].loop);
  return `${heading}<div class="card" id="follow-settings"><div class="card-heading"><h3>거리와 이동 동작</h3><button class="text-button" data-action="remove-follow">설정 제거</button></div>
    ${check("새로 설치할 때 기본으로 켜기", `${path}.enabled`)}
    <div class="form-grid">${field("마우스 앞에서 멈출 거리 (DIP)", `${path}.stopDistanceDip`, "number", 'min="0" max="2000" step="1"')}
    <label class="field"><span>따라오기 우선순위 (0~100)</span><input id="follow-priority" type="number" min="0" max="100" step="1" placeholder="기본 순서" value="${config.priority ?? ""}"></label></div>
    <p class="hint">숫자가 클수록 우선하고, 같으면 이벤트가 먼저 표시됩니다. 비워 두면 일회성 이벤트가 먼저 표시되고, 지속 이벤트보다 따라오기가 우선합니다.</p>
    <p class="hint">0이면 캐릭터 중심이 포인터까지 이동합니다. 앱의 설정이나 우클릭 메뉴에서 켜고 끌 수 있습니다. 다른 화면에 있는 포인터는 따라가지 않습니다.</p>
    ${config.bands.map((band, index) => {
      const key = `${path}.bands.${index}`;
      const current = band.animation ?? "";
      return `<div class="form-grid">${field("이 거리부터 적용 (DIP)", `${key}.minDistanceDip`, "number", `min="0" max="10000" step="1" ${index === 0 ? "readonly" : ""}`)}${field("속도 (DIP/초)", `${key}.speedDipPerSecond`, "number", 'min="1" max="1000" step="1"')}</div><label class="field"><span>반복 애니메이션</span><select data-follow-band="${index}">${option("", "기본 대기 모습", current)}${looping.map(name => option(name, name, current)).join("")}</select></label>${index > 0 ? `<button class="text-button" data-action="remove-follow-band" data-id="${index}">이 구간 삭제</button>` : ""}`;
    }).join("")}
    <p class="hint">현재 거리가 여러 구간에 해당하면 가장 먼 구간부터 적용합니다. 예를 들어 가까우면 걷고 180 DIP부터 뛰도록 설정할 수 있습니다.</p>
    <button class="small" data-action="add-follow-band" ${config.bands.length >= 8 ? "disabled" : ""}>＋ 거리 구간</button></div>`;
}
const cursorDirections: [CursorDirection, string][] = [
  ["upLeft", "↖ 왼쪽 위"], ["up", "↑ 위"], ["upRight", "↗ 오른쪽 위"],
  ["left", "← 왼쪽"], ["center", "· 가운데"], ["right", "→ 오른쪽"],
  ["downLeft", "↙ 왼쪽 아래"], ["down", "↓ 아래"], ["downRight", "↘ 오른쪽 아래"],
];
function idleAppearanceOptions(current: string | undefined, inherited = false) {
  const resources = doc!.character.resources;
  const imageName = (id: string) => resourceName(id, resources[id]);
  const selected = current?.startsWith("resource:") ? current : current ? `clip:${current}` : "";
  const loops = clips().filter((name) => doc!.character.animations[name].loop);
  return (inherited ? option("", "기본 대기 모습 사용", selected) : "")
    + `<optgroup label="반복 애니메이션">${loops.map((name) => {
      const resource = doc!.character.animations[name].resource;
      const label = resource && resources[resource] ? `${name} · ${imageName(resource)}` : name;
      return option(`clip:${name}`, label, selected);
    }).join("")}</optgroup>`
    + (Object.keys(resources).length ? `<optgroup label="이미지·GIF 바로 선택">${Object.keys(resources).map((id) =>
      option(`resource:${id}`, imageName(id), selected),
    ).join("")}</optgroup>` : "");
}
function selectIdleAppearance(value: string) {
  if (value.startsWith("clip:")) return value.slice(5);
  const resource = value.slice("resource:".length);
  if (!Object.hasOwn(doc!.character.resources, resource)) throw new Error("프로젝트에 있는 이미지나 GIF를 선택하세요.");
  const animations = doc!.character.animations;
  const reusable = Object.entries(animations).find(([, animation]) =>
    animation.resource === resource && animation.loop && animation.fps === 12
    && animation.frames.length === 0 && animation.regions.length === 0 && animation.useSourceTiming,
  );
  if (reusable) return reusable[0];
  if (Object.keys(animations).length >= 128)
    throw new Error("동작이 128개입니다. 기존 동작을 선택하거나 사용하지 않는 동작을 정리하세요.");
  const prefix = `idle-${resource}`.slice(0, 58);
  let name = prefix;
  for (let index = 2; Object.hasOwn(animations, name); index++) name = `${prefix}-${index}`;
  animations[name] = { resource, frames: [], regions: [], fps: 12, loop: true, useSourceTiming: true };
  return name;
}
function idleGroupName(group: IdleGroup) {
  return group.name?.trim() || group.states.map(phase => timerPhaseLabels[phase]).join(" · ") || "새 대기 모습";
}
function currentIdleGroup() {
  return idleGroupDraft?.id === selectedIdleGroup ? idleGroupDraft
    : doc?.project.idleGroups?.find(group => group.id === selectedIdleGroup);
}
function idleStateOwner(phase: TimerPhase) {
  return doc!.project.idleGroups?.find(group => group.states.includes(phase))
    ?? (idleGroupDraft?.states.includes(phase) ? idleGroupDraft : undefined);
}
function idleDirectionsEditor(group: IdleGroup | undefined) {
  const idle = doc!.project.idle;
  const custom = group?.cursorMode === "custom";
  const directions = custom ? group.directions : idle?.directions;
  const enabled = custom || (group?.cursorMode !== "off" && idle?.cursorEnabled);
  const pose = directions?.[selectedIdleDirection];
  const mirror = pose?.mirror == null ? "" : String(pose.mirror);
  const readonly = group && !custom ? "disabled" : "";
  const activeDirection = cursorDirections.find(([direction]) => direction === selectedIdleDirection)![1];
  return `<div class="card"><h3>커서 방향별 모습</h3>
    ${group ? `<label class="field"><span>방향 설정</span><select id="idle-group-cursor-mode">${option("inherit", "기본 대기 방향 설정 사용", group.cursorMode)}${option("custom", "방향별로 직접 설정", group.cursorMode)}${option("off", "사용 안 함", group.cursorMode)}</select></label>`
      : `<label class="toggle"><input type="checkbox" id="idle-cursor-enabled" ${idle?.cursorEnabled ? "checked" : ""}><span>커서 위치에 따라 모습 바꾸기</span></label>`}
    <p class="hint">${group?.cursorMode === "off" ? "커서 위치와 관계없이 묶음의 기본 모습을 사용합니다."
      : group?.cursorMode === "inherit" && !enabled ? "기본 대기에서 커서 방향 설정이 꺼져 있습니다."
      : "Player에서 마우스 감지가 켜져 있을 때 적용합니다. 리소스가 비어 있는 방향은 편집 중인 기본 모습을 사용합니다."}</p>
    ${enabled ? `<div class="cursor-directions idle-direction-picker" role="group" aria-label="커서 방향 선택">${cursorDirections.map(([direction, label]) =>
      `<button class="small" data-action="idle-direction" data-id="${direction}" aria-pressed="${direction === selectedIdleDirection}">${label}${directions?.[direction] ? " · 설정됨" : ""}</button>`).join("")}</div>
      <fieldset class="cursor-direction"><legend>${activeDirection}</legend><div class="form-grid">
      <label class="field"><span>애니메이션 또는 리소스</span><select data-idle-pose-field="animation" ${readonly}>${idleAppearanceOptions(pose?.animation ?? undefined, true)}</select></label>
      <label class="field"><span>좌우 방향</span><select data-idle-pose-field="mirror" ${readonly}>${option("", "자동 (커서 방향)", mirror)}${option("false", "원본", mirror)}${option("true", "좌우 반전", mirror)}</select></label></div>
      <button class="small" data-action="test-cursor" data-id="${selectedIdleDirection}" ${(group && group === idleGroupDraft) || !doc!.project.defaults.input.mouse ? "disabled" : ""}>이 방향 미리보기</button></fieldset>` : ""}</div>`;
}
function idleSettings() {
  if (!doc!.character.defaultAnimation) return firstSpriteCard();
  const group = currentIdleGroup();
  const isDraft = !!group && group === idleGroupDraft;
  const groups = doc!.project.idleGroups ?? [];
  const items = [...groups, ...(idleGroupDraft ? [idleGroupDraft] : [])];
  const unassigned = timerPhases.filter(phase => !groups.some(group => group.states.includes(phase)));
  const phase = selectedIdlePhase && group?.states.includes(selectedIdlePhase) ? selectedIdlePhase : group?.states[0];
  return `${title("IDLE STATE", "대기 상태", "같은 모습을 사용할 상태를 하나로 묶을 수 있습니다.")}
    <div class="card"><div class="card-heading"><h3>대기 모습</h3><button class="small" data-action="add-idle-group" ${unassigned.length || idleGroupDraft ? "" : "disabled"}>＋ 대기 모습 추가</button></div>
    <div class="binding-list idle-group-list">
      <button class="binding-row ${!group ? "active" : ""}" data-action="select-idle-group" aria-pressed="${!group}"><span><strong>기본 대기</strong></span><small>필수</small></button>
      ${items.map(item => `<button class="binding-row ${item === group ? "active" : ""}" data-action="select-idle-group" data-id="${esc(item.id)}" aria-pressed="${item === group}"><span><strong>${esc(idleGroupName(item))}</strong></span><small>${item === idleGroupDraft ? "추가 중" : item.states.map(state => timerPhaseLabels[state]).join(" · ")}</small></button>`).join("")}
    </div><p class="hint">${unassigned.length ? "기본 대기 적용 · " + unassigned.map(state => timerPhaseLabels[state]).join(", ") : "모든 상태에 대기 모습이 지정되어 있습니다."}</p></div>
    <div class="card"><div class="card-heading"><h3 id="idle-group-heading">${group ? esc(idleGroupName(group)) : "기본 대기"}</h3>
    ${group && !isDraft ? '<button class="text-button danger" data-action="remove-idle-group">묶음 삭제</button>' : ""}</div>
    ${group ? `<label class="field"><span>묶음 이름 (선택)</span><input id="idle-group-name" maxlength="40" value="${esc(group.name)}" placeholder="예: 휴식 포즈"></label>
    <fieldset><legend>적용할 상태</legend><div class="form-grid">${timerPhases.map(state => {
      const owner = idleStateOwner(state);
      const checked = group.states.includes(state);
      const other = owner && owner !== group;
      const disabled = other || (!isDraft && checked && group.states.length === 1);
      return `<div><label class="toggle"><input type="checkbox" data-idle-state="${state}" ${checked ? "checked" : ""} ${disabled ? "disabled" : ""} ${other ? `aria-describedby="idle-owner-${state}"` : ""}><span>${timerPhaseLabels[state]}</span></label>${other ? `<small id="idle-owner-${state}" class="idle-state-owner">${esc(idleGroupName(owner))}${owner === idleGroupDraft ? "에서 추가 중" : "에서 사용 중"}</small>` : ""}</div>`;
    }).join("")}</div><p class="hint">${!isDraft && group.states.length === 1 ? "마지막 상태를 해제하려면 묶음을 삭제하세요." : "상태마다 대기 모습을 하나만 연결할 수 있습니다."}</p></fieldset>` : ""}
    <label class="field"><span>기본 모습</span><select id="idle-appearance">${idleAppearanceOptions(group ? group.animation ?? undefined : doc!.character.defaultAnimation, !!group)}</select></label>
    <p class="hint">${group?.states.includes("countdown") ? "일반 타이머 모습은 바로가기와 사용자 지정 타이머에 공통으로 적용됩니다." : group ? "" : "묶음이 없는 상태와 타이머를 일시정지하거나 종료했을 때 이 모습을 사용합니다."}</p>
    ${isDraft ? `<div class="row-actions"><button class="small primary" data-action="commit-idle-group" ${group.states.length ? "" : "disabled"}>묶음 추가</button><button class="small" data-action="discard-idle-group">취소</button></div>`
      : `${group && group.states.length > 1 ? `<label class="field"><span>미리 볼 상태</span><select id="idle-preview-phase">${group.states.map(state => option(state, timerPhaseLabels[state], phase ?? "")).join("")}</select></label>` : ""}
      <button class="small" data-action="test-idle">${group ? "묶음 미리보기" : "기본 대기 미리보기"}</button>`}
    ${group && !doc!.project.timer?.enabled ? '<p class="hint">타이머 상태를 미리 보고 Player에 적용하려면 프로젝트에서 타이머를 켜세요. <button class="small" data-action="enable-project-timer">타이머 켜기</button></p>' : ""}</div>
    ${idleDirectionsEditor(group)}`;
}
function soundSettings(binding: Binding) {
  const sounds = Object.entries(doc!.character.sounds);
  const volume = Math.round((binding.sound?.volume ?? 0.7) * 100);
  return `<div class="form-grid"><label class="field"><span>이벤트 사운드</span><select id="event-sound">${option("", "재생 안 함", binding.sound?.resource ?? "")}${sounds.map(([id, sound]) => option(id, resourceName(id, sound), binding.sound?.resource ?? "")).join("")}</select></label>
    ${binding.sound ? `<label class="field"><span>볼륨 (%)</span><input id="event-volume" type="number" min="0" max="100" step="1" value="${volume}"></label>` : ""}</div>
    <p class="hint">${sounds.length ? "이 동작이 시작될 때 사운드를 한 번 재생합니다. 다른 동작으로 바뀌면 소리도 멈춥니다." : "리소스에서 WAV 또는 MP3를 가져오면 이벤트에 연결할 수 있습니다."}</p>`;
}
function reactionSettings(binding: Binding, path: string) {
  const effect = binding.effect;
  if (effect.type === "faceCursor") return "";
  const cursor = check("이 동작 중 커서 위치에 따라 좌우 반전", `${path}.followCursor`)
    + (binding.trigger.type === "drag" ? "" : '<p class="hint">좌우 반전을 사용하려면 버디 앱에서 마우스 감지를 켜세요.</p>');
  const sound = soundSettings(binding);
  if (effect.type !== "playAnimation") {
    return `${cursor}${sound}${binding.trigger.type === "drag" ? "" : `<label class="field"><span>우선순위 (0~100, 비우면 기본 순서)</span><input id="continuous-priority" type="number" min="0" max="100" value="${effect.priority ?? ""}"></label>`}`;
  }
  if (automaticTrigger(binding.trigger.type)) {
    const cooldown = binding.trigger.type === "typingStopped" ? "" : field("재반응 대기 (ms)", `${path}.effect.cooldownMs`, "number", 'min="0" max="10000"');
    return `${cursor}${sound}<div class="form-grid">${field("자동 이벤트 우선순위 (0~100)", `${path}.effect.priority`, "number", 'min="0" max="100"')}${cooldown}<label class="field"><span>표시 시간 (ms, 비우면 동작 길이)</span><input id="duration-ms" type="number" min="1" max="60000" value="${effect.durationMs ?? ""}"></label></div><p class="hint">입력 반응이나 이동이 시작되면 자동 동작을 끝냅니다.</p>`;
  }
  const policy = effect.interruption ?? "byPriority";
  const finish = policy === "finish";
  return `${cursor}${sound}<div class="form-grid">${field("우선순위 (0~100)", `${path}.effect.priority`, "number", 'min="0" max="100"')}${field("재반응 대기 (ms)", `${path}.effect.cooldownMs`, "number", 'min="0" max="10000"')}
    <label class="field"><span>새 이벤트가 들어오면</span><select id="interruption-policy">${option("byPriority", "우선순위에 따라 전환", policy)}${option("replace", "즉시 전환", policy)}${option("finish", "재생을 마칠 때까지 유지", policy)}</select></label>
    <label class="field"><span>표시 시간 (ms, 비우면 동작 길이)</span><input id="duration-ms" type="number" min="1" max="60000" value="${effect.durationMs ?? ""}"></label>
    ${finish ? "" : `<label class="field"><span>같은 이벤트가 반복되면</span><select data-path="${path}.effect.retrigger">${option("restart", "처음부터 재생", effect.retrigger)}${option("ignore", "현재 재생 유지", effect.retrigger)}</select></label>`}
    </div>${finish ? '<p class="hint">표시 시간이 끝날 때까지 다른 입력이나 같은 이벤트가 재생을 바꾸지 않습니다.</p>' : supportsStopOnRelease(binding.trigger) ? check("키나 버튼을 떼면 재생 중단", `${path}.effect.stopOnRelease`) : ""}`;
}
type KeyboardTrigger = Extract<Binding["trigger"], { code: string }>;
type KeywordTrigger = Extract<Binding["trigger"], { type: "keySequence" }>;
const modifierOptions = [
  { id: "control", label: "Ctrl", flag: "ctrlKey", key: "Control" },
  { id: "alt", label: "Alt", flag: "altKey", key: "Alt" },
  { id: "shift", label: "Shift", flag: "shiftKey", key: "Shift" },
  { id: "meta", label: /Win/.test(navigator.platform) ? "Win" : "Cmd", flag: "metaKey", key: "Meta" },
] as const;
const physicalKeys = new Set(supportedPhysicalKeys);
let pendingModifier: { code: string; modifiers: KeyModifier[] } | undefined;

function keyCombinationLabel({ code, modifiers }: KeyChord) {
  const keyLabel = code === "*" ? "모든 일반 키" : code.replace(/^(Key|Digit)(?=.$)/, "") || "키를 입력하세요";
  const labels = modifierOptions.filter((key) => modifiers?.includes(key.id)).map((key) => key.label);
  return [...labels, keyLabel].join(" + ");
}

const pointerOptions: { input: PointerInput; label: string }[] = [
  { input: { type: "mouseDown", button: "primary" }, label: "왼쪽 버튼 누름" },
  { input: { type: "mouseDown", button: "secondary" }, label: "오른쪽 버튼 누름" },
  { input: { type: "mouseDown", button: "middle" }, label: "가운데 버튼 누름" },
  { input: { type: "mouseUp", button: "primary" }, label: "왼쪽 버튼 뗌" },
  { input: { type: "mouseUp", button: "secondary" }, label: "오른쪽 버튼 뗌" },
  { input: { type: "mouseUp", button: "middle" }, label: "가운데 버튼 뗌" },
  { input: { type: "wheel", direction: "up" }, label: "위로 스크롤" },
  { input: { type: "wheel", direction: "down" }, label: "아래로 스크롤" },
  { input: { type: "wheel", direction: "left" }, label: "왼쪽으로 스크롤" },
  { input: { type: "wheel", direction: "right" }, label: "오른쪽으로 스크롤" },
  { input: { type: "buddyClick" }, label: "버디 클릭" },
];
function pointerInputs(trigger: Binding["trigger"]): PointerInput[] | undefined {
  switch (trigger.type) {
    case "mouseDown":
    case "mouseUp":
    case "wheel":
    case "buddyClick":
      return [trigger];
    case "mouseInput":
      return trigger.inputs;
  }
}
function pointerInputKey(input: PointerInput) {
  return "button" in input ? `${input.type}:${input.button}`
    : input.type === "wheel" ? `wheel:${input.direction}` : input.type;
}
function supportsStopOnRelease(trigger: Binding["trigger"]) {
  const inputs = pointerInputs(trigger);
  return trigger.type === "keyDown" ||
    (!!inputs?.length && inputs.every((input) => input.type === "mouseDown"));
}
function setPointerInputs(binding: Binding, inputs: PointerInput[]) {
  binding.trigger = inputs.length === 1 ? inputs[0] : { type: "mouseInput", inputs };
  if (binding.effect.type === "playAnimation" && !supportsStopOnRelease(binding.trigger)) {
    binding.effect.stopOnRelease = false;
  }
  markDirty();
  layout();
}
function pointerTriggerEditor(inputs: PointerInput[]) {
  const keys = inputs.map(pointerInputKey);
  return `<div class="key-capture-list">
    <div class="card-heading"><strong>마우스·트랙패드 입력</strong><button type="button" class="small" data-action="add-pointer-input" ${inputs.length >= pointerOptions.length ? "disabled" : ""}>＋ 입력 추가</button></div>
    <p class="hint">등록한 입력 중 하나가 들어오면 실행합니다. 스크롤에는 마우스 휠과 트랙패드의 두 손가락 스크롤이 포함됩니다.</p>
    ${inputs.map((input, index) => `<div class="key-capture"><div class="form-grid">
      <label class="field"><span>입력 ${index + 1}</span><select data-pointer-input="${index}">${pointerOptions.map(({ input: choice, label }) => {
        const key = pointerInputKey(choice);
        return keys.includes(key) && key !== keys[index] ? "" : option(key, label, keys[index]);
      }).join("")}</select></label>
      ${inputs.length > 1 ? `<button type="button" class="text-button danger" data-action="remove-pointer-input" data-id="${index}" aria-label="입력 ${index + 1} 삭제">삭제</button>` : ""}
    </div></div>`).join("")}
    ${inputs.some((input) => input.type === "buddyClick") ? '<p class="hint">버디 클릭은 버디 위에서 기본 버튼을 누른 뒤 드래그 없이 놓을 때 실행합니다.</p>' : ""}
    ${inputs.some((input) => input.type === "mouseDown") && inputs.some((input) => input.type !== "mouseDown") ? '<p class="hint">버튼을 뗄 때 재생을 중단하려면 버튼 누름만 등록하세요.</p>' : ""}
  </div>`;
}

type TimerTrigger = Extract<Binding["trigger"], { type: "timerCompleted" | "timerStarted" }>;
function timerTriggerPhases(trigger: TimerTrigger): TimerPhase[] {
  return timerPhases.filter(phase => [trigger.phase, ...(trigger.alternatives ?? [])].some(selected =>
    selected === phase || (selected === "break" && (phase === "shortBreak" || phase === "longBreak"))));
}

function timerTriggerEditor(trigger: TimerTrigger) {
  const selected = timerTriggerPhases(trigger);
  const start = trigger.type === "timerStarted";
  const enable = doc!.project.timer?.enabled ? "" : `<p class="hint">Player에서 타이머 반응을 사용하려면 프로젝트 타이머를 켜고 빌드하세요. <button type="button" class="small" data-action="enable-project-timer">타이머 켜기</button></p>`;
  return `<fieldset><legend>${start ? "시작할 단계" : "완료할 단계"}</legend><div class="form-grid">${timerPhases.map(phase => {
    const checked = selected.includes(phase);
    return `<label class="toggle"><input type="checkbox" data-timer-phase="${phase}" ${checked ? "checked" : ""} ${checked && selected.length === 1 ? "disabled" : ""}><span>${timerPhaseLabels[phase]}</span></label>`;
  }).join("")}</div><p class="hint">${start ? "선택한 단계가 새로 실행될 때 반응합니다. 재개하면 남은 시간을 이어서 셉니다." : "선택한 단계의 시간이 다 되면 반응합니다."} 오른쪽 미리보기에서 단계별로 시험할 수 있습니다.</p>${enable}</fieldset>`;
}

function inputChoices(trigger: Binding["trigger"]): string[] {
  if (trigger.type === "timerCompleted" || trigger.type === "timerStarted") return timerTriggerPhases(trigger).map(phase => timerPhaseLabels[phase]);
  if ("code" in trigger) return [trigger, ...(trigger.alternatives ?? [])].map(keyCombinationLabel);
  if (trigger.type === "keySequence") return [
    trigger.keyword || (trigger.steps?.length ? "직접 지정한 키 순서" : ""),
    ...(trigger.alternatives ?? []).map(({ keyword }) => keyword),
  ];
  return pointerInputs(trigger)?.map((input) =>
    pointerOptions.find((choice) => pointerInputKey(choice.input) === pointerInputKey(input))!.label,
  ) ?? [];
}

function typingStoppedFields(path: string) {
  return `<div class="form-grid">
    ${field("최소 입력 구간 (초)", `${path}.minTypingSeconds`, "number", 'min="2" max="60" step="1"')}
    ${field("최소 키 입력 수", `${path}.minKeys`, "number", 'min="2" max="100" step="1"')}
    ${field("멈춤 판정 (ms)", `${path}.pauseMs`, "number", 'min="500" max="5000" step="1"')}
    ${field("최소 재반응 간격 (초)", `${path}.minIntervalSeconds`, "number", 'min="10" max="600" step="1"')}
  </div><p class="hint">최소 입력 구간과 키 수를 모두 채운 뒤, 지정한 시간만큼 타이핑이 끊기면 반응합니다. 키보드 감지가 필요하며 프로젝트당 하나를 설정할 수 있습니다.</p>`;
}

function typingRepeatFits(trigger: TypingStoppedTrigger) {
  return trigger.minTypingSeconds * 1000 + trigger.pauseMs + 1 < trigger.minIntervalSeconds * 1000;
}

const typingTestLabels = ["기준 충족 후 멈춤", "짧게 입력 후 멈춤", "멈춤 판정 시 입력 재개", "재반응 간격 안에 다시 멈춤"];
function testInputSelector(trigger: Binding["trigger"] | undefined) {
  if (!trigger) return "";
  if (trigger.type === "typingStopped") {
    const fits = typingRepeatFits(trigger);
    return `<label class="field"><span>테스트 상황</span><select id="test-variant">${typingTestLabels.map((label, index) =>
      `<option value="${index}" ${index === 3 && !fits ? "disabled" : ""}>${label}</option>`,
    ).join("")}</select></label>${fits ? "" : '<p class="hint">현재 설정에서는 두 번째 입력 구간을 채우는 동안 재반응 간격이 지납니다.</p>'}`;
  }
  const choices = inputChoices(trigger);
  return choices.length > 1 ? `<label class="field"><span>${trigger.type === "timerCompleted" || trigger.type === "timerStarted" ? "테스트할 단계" : "테스트할 입력"}</span><select id="test-variant">${choices.map((label, index) => option(String(index), label || `입력 ${index + 1}`, "0")).join("")}</select></label>` : "";
}

function keyboardTriggerEditor(trigger: KeyboardTrigger, path: string) {
  const keys = [trigger, ...(trigger.alternatives ?? [])];
  return `<div class="key-capture-list"><div class="card-heading"><strong>키 입력</strong><button type="button" class="small" data-action="add-key-choice" ${keys.length >= 32 ? "disabled" : ""}>＋ 키 추가</button></div>
    <p class="hint">등록한 키 조합 중 하나가 들어오면 이 이벤트를 실행합니다.</p>
    ${keys.map((key, index) => keyboardChoiceEditor(key, index === 0 ? path : `${path}.alternatives.${index - 1}`, index, keys.length > 1, trigger.type !== "whileKeyDown")).join("")}
    ${trigger.type === "keyDown" ? check("키를 길게 눌러 반복되는 입력에도 반응", `${path}.repeat`) : ""}
  </div>`;
}

function keyboardChoiceEditor(key: KeyChord, path: string, index: number, removable: boolean, allowWildcard: boolean) {
  const { code, modifiers } = key;
  const summary = esc(keyCombinationLabel(key) + (modifiers == null ? " · 보조 키 무관" : ""));
  return `<div class="key-capture" data-key-index="${index}">
    <div class="card-heading"><p id="key-capture-status-${index}" data-key-status class="hint" aria-live="polite" data-summary="${summary}">${summary}</p><div class="row-actions">
      <button type="button" class="small" aria-pressed="false" aria-describedby="key-capture-status-${index}" data-action="capture-key" data-id="${index}">키 녹화</button>
      ${removable ? `<button type="button" class="text-button danger" data-action="remove-key-choice" data-id="${index}" aria-label="키 입력 ${index + 1} 삭제">삭제</button>` : ""}
    </div></div>
    <details><summary>키 코드와 보조 키 직접 설정</summary>
      ${field("물리 키 코드", `${path}.code`)}
      <label class="field"><span>보조 키 조건</span><select data-key-modifier-mode>${option("any", "보조 키 무관", modifiers == null ? "any" : "exact")}${option("exact", "지정한 보조 키만", modifiers == null ? "any" : "exact")}</select></label>
      <div class="key-modifier-options">${modifierOptions.map(({ id, label, key }) =>
        `<label class="toggle"><input type="checkbox" data-key-modifier="${id}" ${modifiers?.includes(id) ? "checked" : ""} ${modifiers == null || code.startsWith(key) ? "disabled" : ""}><span>${label}</span></label>`
      ).join("")}</div>
      ${allowWildcard ? '<p class="hint">키 코드에 *를 입력하면 모든 일반 키에 반응합니다.</p>' : ""}
    </details>
  </div>`;
}

function keywordTriggerEditor(trigger: KeywordTrigger, path: string) {
  const choices = [trigger, ...(trigger.alternatives ?? [])];
  return `<div class="key-capture-list"><div class="card-heading"><strong>키워드 입력</strong><button type="button" class="small" data-action="add-keyword-choice" ${choices.length >= 32 ? "disabled" : ""}>＋ 키워드 추가</button></div>
    <p class="hint">등록한 키워드 중 하나를 입력하면 이벤트가 실행됩니다. 영문자·숫자·공백을 쓸 수 있고, 한글 키워드는 한글 두벌식을 선택하세요.</p>
    ${choices.map(({ layout }, index) => {
      const choice = index === 0 ? path : `${path}.alternatives.${index - 1}`;
      const remove = choices.length > 1 ? `<button type="button" class="text-button danger" data-action="remove-keyword-choice" data-id="${index}">키워드 ${index + 1} 삭제</button>` : "";
      return `<div class="key-capture">${index === 0 && trigger.steps?.length ? '<p class="hint">직접 지정한 물리 키 순서</p>' :
        `<div class="form-grid">${field(`키워드 ${index + 1}`, `${choice}.keyword`)}<label class="field"><span>키 배열</span><select data-path="${choice}.layout">${option("ko-KR-2set", "한글 두벌식", layout ?? "ko-KR-2set")}${option("en-US-qwerty", "영문 QWERTY", layout ?? "ko-KR-2set")}</select></label></div>`}${remove}</div>`;
    }).join("")}
    ${field("입력 제한 시간 (ms)", `${path}.timeoutMs`, "number", 'min="500" max="10000"')}
  </div>`;
}

function captureButton(index: number) {
  return document.querySelector<HTMLButtonElement>(`[data-action="capture-key"][data-id="${index}"]`);
}

function setKeyCapture(button: HTMLButtonElement, active: boolean) {
  pendingModifier = undefined;
  const status = button.closest(".key-capture")?.querySelector<HTMLElement>("[data-key-status]");
  if (!status) return;
  button.setAttribute("aria-pressed", String(active));
  button.textContent = active ? "녹화 취소" : "키 녹화";
  status.textContent = active ? "등록할 키나 키 조합을 눌러주세요." : status.dataset.summary!;
  if (active) button.focus();
}

function saveCapturedKey(button: HTMLButtonElement, code: string, modifiers: KeyModifier[]) {
  const trigger = doc?.project.bindings[selectedBinding]?.trigger;
  if (!trigger || !("code" in trigger)) return;
  const index = Number(button.dataset.id);
  const key = index === 0 ? trigger : trigger.alternatives?.[index - 1];
  if (!key) return;
  setKeyCapture(button, false);
  key.code = code;
  key.modifiers = modifiers;
  markDirty();
  layout();
  captureButton(index)?.focus();
}

type OneShotEffect = Extract<Effect, { type: "playAnimation" }>;
type MotionCandidate = { animation: string; weight: number };
function motionClips(effect: OneShotEffect) {
  return [effect.animation, ...(effect.variation?.alternatives ?? [])];
}
function motionCandidates(effect: OneShotEffect): MotionCandidate[] {
  return motionClips(effect).map((animation, index) => ({
    animation, weight: effect.variation?.weights?.[index] ?? 1,
  }));
}
function setMotionCandidates(effect: OneShotEffect, candidates: MotionCandidate[]) {
  const [first, ...alternatives] = candidates;
  effect.animation = first.animation;
  if (alternatives.length) {
    const previous = effect.variation;
    effect.variation = {
      mode: previous?.mode ?? "cycle",
      alternatives: alternatives.map(candidate => candidate.animation),
      ...(previous?.weights || previous?.mode === "weighted"
        ? { weights: candidates.map(candidate => candidate.weight) } : {}),
    };
  }
  else delete effect.variation;
  markDirty();
  layout();
}
function motionWeightTotal(weights: number[]) {
  return weights.every(weight => Number.isInteger(weight) && weight >= 0 && weight <= 10_000)
    ? weights.reduce((sum, weight) => sum + weight, 0) : 0;
}
function motionProbability(weight: number, total: number) {
  if (!total) return "—";
  const percentage = weight / total * 100;
  return percentage > 0 && percentage < 0.01 ? "0.01% 미만"
    : percentage.toLocaleString("ko-KR", { maximumFractionDigits: 2 }) + "%";
}
const motionWeightWarning = "비중은 0~10,000의 정수로 입력하고, 최소 한 모션에 1 이상을 지정하세요.";
function motionEditor(effect: OneShotEffect, names: string[]) {
  const motions = motionClips(effect);
  const weighted = effect.variation?.mode === "weighted";
  const weights = motionCandidates(effect).map(candidate => candidate.weight);
  const total = motionWeightTotal(weights);
  const canAdd = motions.length < 8 && names.includes(effect.animation) && names.some(name => !motions.includes(name));
  const fields = effect.variation ? `<label class="field"><span>재생 방식</span><select id="motion-mode">${option("cycle", "발동할 때마다 순서대로", effect.variation.mode)}${option("shuffle", "섞어서 한 번씩", effect.variation.mode)}${option("weighted", "가중치 랜덤", effect.variation.mode)}</select></label>
  ${weighted ? `<p class="hint">매번 비중에 따라 후보를 하나 뽑습니다. 0은 선택하지 않으며, 같은 모션이 연속으로 나올 수 있습니다.</p><p class="hint" id="motion-weight-warning" ${total ? "hidden" : ""}>${motionWeightWarning}</p>` : ""}
  ${motions.map((current, index) => {
    const choices = names.filter(name => name === current || !motions.includes(name));
    const weight = weighted ? `<label class="field"><span>비중 · <output data-motion-probability>${motionProbability(weights[index], total)}</output></span><input type="number" min="0" max="10000" step="1" value="${weights[index]}" data-motion-weight="${index}" aria-label="모션 ${index + 1} 비중"></label>` : "";
    return `<div class="form-grid"><label class="field"><span>모션 ${index + 1}</span><select data-motion-index="${index}">${choices.includes(current) ? "" : '<option value="" selected disabled>애니메이션 선택</option>'}${choices.map(name => option(name, name, current)).join("")}</select></label>${weight}<div class="row-actions"><button class="small" data-action="move-motion" data-id="${index}:-1" ${index === 0 ? "disabled" : ""}>↑ 위로</button><button class="small" data-action="move-motion" data-id="${index}:1" ${index === motions.length - 1 ? "disabled" : ""}>↓ 아래로</button><button class="text-button danger" data-action="remove-motion" data-id="${index}">빼기</button></div></div>`;
  }).join("")}<p class="hint">이벤트가 실행될 때 후보 하나를 재생합니다. 후보들은 우선순위, 재반응 대기와 사운드를 공유합니다.</p>` : "";
  return fields + `<button class="small" data-action="add-motion" ${canAdd ? "" : "disabled"}>＋ 모션</button>${!canAdd && motions.length < 8 ? '<p class="hint">다른 일회성 애니메이션이 있으면 후보를 추가할 수 있습니다.</p>' : ""}`;
}

function bindingAnimationEditor(effect: Exclude<Effect, { type: "faceCursor" }>, path: string) {
  const once = effect.type === "playAnimation";
  const names = clips().filter(name => doc!.character.animations[name].loop !== once);
  if (once && effect.variation) return motionEditor(effect, names);
  const current = effect.animation ?? "";
  const valid = names.includes(current);
  const canUseIdle = effect.type === "move";
  const placeholder = canUseIdle
    ? option("", "기본 대기 모습", current)
    : `<option value="" disabled ${valid ? "" : "selected"}>애니메이션 선택</option>`;
  const hint = !canUseIdle && !names.length
    ? `<p class="hint">${once ? "반복 재생이 꺼진" : "반복 재생이 켜진"} 애니메이션이 필요합니다. <button class="text-button" data-action="tab" data-id="animations">애니메이션 편집</button></p>`
    : "";
  return `<label class="field"><span>표시할 애니메이션</span><select data-path="${path}.effect.animation">${placeholder}${names.map(name => option(name, name, current)).join("")}</select></label>${hint}${once ? motionEditor(effect, names) : ""}`;
}
function bindingEditor(b: Binding) {
  const pointer = pointerInputs(b.trigger);
  const t: any = b.trigger,
    e: any = b.effect,
    p = `project.bindings.${selectedBinding}`;
  const choices = Object.entries(triggerNames);
  const group = (label: string, automatic: boolean) => `<optgroup label="${label}">${choices.filter(([type]) => automaticTrigger(type) === automatic).map(([type, name]) => option(type, name, triggerCategory(b.trigger))).join("")}</optgroup>`;
  const idleDuration = b.trigger.type === "idleFor"
    ? `${field("대기 시간 (초)", `${p}.trigger.seconds`, "number", 'min="1" max="3600" step="1"')}<p class="hint">시선이나 대기 이미지가 바뀌어도 계속 셉니다. 실제 이동이나 입력 반응 뒤에는 처음부터 셉니다.</p>`
    : "";
  const typingFields = b.trigger.type === "typingStopped" ? typingStoppedFields(`${p}.trigger`) : "";
  const timerFields = b.trigger.type === "timerCompleted" || b.trigger.type === "timerStarted" ? timerTriggerEditor(b.trigger) : "";
  const holdFields = b.trigger.type === "buddyLongPress"
    ? `<label class="field"><span>최소 누르기 시간 (초)</span><input id="buddy-hold-seconds" type="number" min="${buddyHoldMs.min / 1000}" max="${buddyHoldMs.max / 1000}" step="0.001" value="${b.trigger.holdMs / 1000}"></label><p class="hint">버디 위에서 기본 버튼을 제자리에서 길게 누르고 놓으면 반응합니다.</p>` : "";
  return `<div class="card"><div class="card-heading"><h3>이벤트 편집</h3><div class="row-actions"><button class="small" data-action="reorder-binding" data-id="-1" ${selectedBinding === 0 ? "disabled" : ""}>↑ 위로</button><button class="small" data-action="reorder-binding" data-id="1" ${selectedBinding === doc!.project.bindings.length - 1 ? "disabled" : ""}>↓ 아래로</button><button class="text-button danger" data-action="remove-binding">삭제</button></div></div><div class="form-grid">${field("이벤트 ID", `${p}.id`)}<label class="field"><span>발생 조건</span><select id="trigger-type">${group("입력과 버디 조작", false)}${group("자동 반응", true)}</select></label></div>${idleDuration}${typingFields}${timerFields}${holdFields}${"code" in b.trigger ? keyboardTriggerEditor(b.trigger, `${p}.trigger`) : ""}${t.type === "typingRate" ? `<div class="form-grid">${field("최소 입력 속도 (키/초)", `${p}.trigger.minKps`, "number", 'min="0.5" max="1000" step="any"')}<label class="field"><span>최대 입력 속도 (비우면 제한 없음)</span><input id="max-kps" type="number" min="0.5" max="1000" step="any" value="${t.maxKps ?? ""}"></label></div><p class="hint">최소값 이상, 최대값 미만에 적용합니다. 속도 구간은 서로 겹칠 수 없습니다.</p>` : ""}${b.trigger.type === "keySequence" ? keywordTriggerEditor(b.trigger, `${p}.trigger`) : ""}${pointer ? pointerTriggerEditor(pointer) : ""}${t.type === "whileKeyDown" ? `<label class="field"><span>동작 종류</span><select id="held-effect">${option("sustainAnimation", "키를 누르는 동안 애니메이션", e.type)}${option("move", "키를 누르는 동안 이동", e.type)}</select></label>` : ""}${
    b.effect.type !== "faceCursor"
      ? bindingAnimationEditor(b.effect, p)
      : ""
  }${reactionSettings(b, p)}${e.type === "move" ? `<div class="form-grid">${field("X 방향 (-1~1)", `${p}.effect.x`, "number", 'min="-1" max="1" step="0.1"')}${field("Y 방향 (-1~1)", `${p}.effect.y`, "number", 'min="-1" max="1" step="0.1"')}${field("이동 속도 (DIP/초)", `${p}.effect.speedDipPerSecond`, "number", 'min="1" max="1000"')}</div>` : ""}</div>`;
}
function timing() {
  return `${title("RESPONSE TIMING", "반응 타이밍", "키 입력 속도를 측정하는 방법과 애니메이션 전환 시간을 조절하세요.")}<div class="timing-highlight"><span class="timing-icon">◷</span><div><h3>입력 속도 애니메이션이 끝나는 시점</h3><p>마지막 일반 키 입력 후 ${doc!.project.defaults.timing.idleTimeoutMs}ms에 속도 애니메이션을 끝냅니다.</p></div><strong>${doc!.project.defaults.timing.idleTimeoutMs}<small>ms</small></strong></div><div class="card">${field("입력 중단 (ms)", "project.defaults.timing.idleTimeoutMs", "number", 'min="50" max="5000" step="10"')}<p class="hint">기본 250ms. 더 빠른 복귀는 150~200ms, 여유 있는 복귀는 400~600ms를 사용해보세요. 일회성 동작의 길이는 이벤트의 표시 시간에서 조절합니다.</p><div class="form-grid">${field("속도 측정 구간 (ms)", "project.defaults.timing.rateWindowMs", "number", 'min="100" max="10000" step="50"')}${field("속도 평가 간격 (ms)", "project.defaults.timing.rateEvaluationMs", "number", 'min="10" max="250" step="10"')}${field("속도 구간 안정 시간 (ms)", "project.defaults.timing.rateStabilityMs", "number", 'min="0" max="2000" step="10"')}</div><p class="hint">속도 = 측정 구간의 키 입력 수 ÷ 구간(초). 안정 시간이 지나야 새 속도 구간으로 바뀝니다. 입력 중단 시간이 지나면 속도 애니메이션은 안정 시간을 기다리지 않고 끝납니다.</p></div><div class="card"><h3>반응 타이밍 프리셋</h3><div class="preset-row"><button data-action="timing-preset" data-id="snappy">빠른 전환 <small>중단 150ms · 안정 50ms</small></button><button data-action="timing-preset" data-id="balanced">기본 <small>중단 250ms · 안정 100ms</small></button><button data-action="timing-preset" data-id="relaxed">느긋한 전환 <small>중단 500ms · 안정 200ms</small></button></div></div>`;
}
const defaultMacDmg = (): MacosDmg => structuredClone(macDmgPreset);
const macDmgDefaultAssets = {
  background: new URL("../../../packaging/macos/defaults/background.png", import.meta.url).href,
  guide: new URL("../../../packaging/macos/defaults/installation-guide.pdf", import.meta.url).href,
};
const windowsDefaultGuide = new URL("../../../packaging/windows/defaults/installation-guide.pdf", import.meta.url).href;
const defaultDmgField = (slot: "background" | "guide") =>
  slot === "background" ? "useDefaultBackground" : "useDefaultGuide";
function initialGuidePosition(dmg: MacosDmg) {
  const xCenter = Math.floor(dmg.width / 2), bottom = dmg.height - 128;
  const fits = (x: number, y: number) =>
    x >= 72 && x <= dmg.width - 72 && y >= 72 && y <= bottom &&
    [dmg.appPosition, dmg.applicationsPosition].every(icon =>
      Math.abs(x - icon.x) >= 132 || Math.abs(y - icon.y) >= 132);
  const left = 2 * dmg.appPosition.x - dmg.applicationsPosition.x;
  if (dmg.useDefaultGuide && dmg.appPosition.y === dmg.applicationsPosition.y &&
      left < dmg.appPosition.x && fits(left, dmg.appPosition.y)) {
    return { x: left, y: dmg.appPosition.y };
  }
  for (const y of [bottom, 72, Math.floor(dmg.height / 2)]) {
    for (const x of [xCenter, 72, dmg.width - 72]) {
      if (fits(x, y)) return { x, y };
    }
  }
  return { x: xCenter, y: Math.max(72, bottom) };
}
function installAsset(dmg: MacosDmg, slot: "background" | "guide", label: string, accept: string) {
  const path = dmg[slot];
  const filename = path?.split("/").at(-1);
  const builtin = !path && dmg[defaultDmgField(slot)];
  const description = filename ? esc(doc!.library[filename]?.originalName ?? filename) : builtin ? (slot === "guide" ? "기본 PDF" : "기본 배경") : "macOS에서 제외";
  return `<div class="library-row"><div><strong>${label}</strong><small> · ${description}</small></div><div class="row-actions">
    ${builtin ? `<a class="button small" href="${macDmgDefaultAssets[slot]}" target="_blank" rel="noopener">macOS 기본 파일 보기</a>` : `<button class="text-button" data-action="restore-install-asset" data-id="${slot}">기본 파일 사용</button>`}
    <label class="button small">${path || builtin ? "교체" : "파일 선택"}<input type="file" data-install-asset="${slot}" accept="${accept}" hidden></label>
    ${path || builtin ? `<button class="text-button" data-action="remove-install-asset" data-id="${slot}">macOS에서 제외</button>` : ""}</div></div>`;
}
function installerSettings() {
  const custom = doc!.project.build.macosDmg;
  const dmg = custom ?? defaultMacDmg();
  const p = "project.build.macosDmg";
  return `<div class="card"><h3>설치 가이드</h3>
    ${installAsset(dmg, "guide", "가이드 파일 (PDF·HTML·PNG·JPEG)", ".pdf,.html,.png,.jpg,.jpeg")}
    <p class="hint">macOS DMG와 Windows 설치 패키지에 각 운영체제의 가이드를 넣습니다. 직접 올린 파일은 두 플랫폼에서 사용합니다. macOS에서 가이드를 제외하면 Windows는 <a href="${windowsDefaultGuide}" target="_blank" rel="noopener">기본 가이드</a>를 사용합니다.</p></div>
    <div class="card" id="mac-dmg-settings"><div class="card-heading"><h3>macOS 설치 화면</h3>${custom ? '<button class="text-button" data-action="remove-dmg">배경·가이드·배치 초기화</button>' : ""}</div>
    ${installAsset(dmg, "background", "DMG 배경 (PNG·JPEG)", ".png,.jpg,.jpeg")}
    ${custom ? `<div class="form-grid">${field("창 너비", `${p}.width`, "number", 'min="400" max="1600" step="1"')}${field("창 높이", `${p}.height`, "number", 'min="260" max="1200" step="1"')}${field("앱 아이콘 X", `${p}.appPosition.x`, "number", 'min="72" step="1"')}${field("앱 아이콘 Y", `${p}.appPosition.y`, "number", 'min="72" step="1"')}${field("Applications 아이콘 X", `${p}.applicationsPosition.x`, "number", 'min="72" step="1"')}${field("Applications 아이콘 Y", `${p}.applicationsPosition.y`, "number", 'min="72" step="1"')}</div>
    ${dmg.guide || dmg.useDefaultGuide ? `<div class="form-grid"><label class="field"><span>가이드 아이콘 배치</span><select id="dmg-guide-position-mode">${option("auto", "자동 배치", dmg.guidePosition ? "manual" : "auto")}${option("manual", "직접 지정", dmg.guidePosition ? "manual" : "auto")}</select></label>${dmg.guidePosition ? `${field("가이드 아이콘 X", `${p}.guidePosition.x`, "number", `min="72" max="${dmg.width - 72}" step="1"`)}${field("가이드 아이콘 Y", `${p}.guidePosition.y`, "number", `min="72" max="${dmg.height - 128}" step="1"`)}` : ""}</div>` : ""}
    <p class="hint">좌표는 창의 왼쪽 위에서 잽니다. 기본 배경은 기본 창 크기와 아이콘 배치에 맞춰져 있습니다.</p>` : '<button class="small" data-action="configure-dmg">창 크기와 아이콘 위치 조정</button>'}</div>`;
}
function hasBothBuildTargets(targets: Target[]) {
  return targets.includes("macos-arm64") && targets.includes("windows-x64");
}
function builds() {
  const ready = !!doc!.character.defaultAnimation;
  const platform = config.target === "macos-arm64" ? "macOS" : "Windows";
  const targets = doc!.project.build.targets;
  const canBuildLocally = ready && targets.includes(config.target);
  const canBuildBoth = hasBothBuildTargets(targets);
  return `${title("BUILD & HISTORY", "빌드와 설치 파일", "설치 파일을 만들고 이전 빌드 결과를 확인하세요.")}${ready ? "" : firstSpriteCard()}<div class="platform-grid">${[
    ["macos-arm64", "macOS · Apple Silicon", "DMG 설치 파일"],
    ["windows-x64", "Windows · x64", "설치 파일과 가이드가 담긴 ZIP"],
  ].map(([target, label, format]) => {
    const enabled = targets.includes(target as Target);
    const local = enabled && config.target === target;
    return `<div class="platform-card ${local ? "current" : ""}"><span class="pill">${!enabled ? "구성에서 제외됨" : !ready ? "스프라이트 필요" : local ? "이 컴퓨터에서 빌드 가능" : github.ready && canBuildBoth ? "Actions에서 빌드 가능" : canBuildBoth ? "GitHub 연결 필요" : "다른 운영체제에서 빌드"}</span><h3>${label}</h3><p>${format}</p></div>`;
  }).join("")}</div>${installerSettings()}<section id="github-panel">${githubPanel()}</section><div class="build-banner"><div><h3>macOS + Windows 함께 빌드</h3><p>${!canBuildBoth ? "두 플랫폼이 모두 빌드 대상인 프로젝트에서 사용할 수 있습니다." : github.connected ? `저장한 구성, 사용한 리소스와 빌드 엔진을 ${esc(github.repository)}에 업로드합니다. ${github.private ? "비공개 저장소입니다." : "공개 저장소에 업로드한 파일은 누구나 볼 수 있습니다."} 진행 상황과 결과는 아래 빌드 기록에서 확인할 수 있습니다.` : "GitHub 저장소를 연결하면 두 플랫폼용 설치 파일을 한 번에 만들 수 있습니다."}</p></div><button class="primary" data-action="build-remote" ${ready && github.connected && github.ready && canBuildBoth && !doc!.meta.archived ? "" : "disabled"}>업로드하고 두 플랫폼 빌드 ↗</button></div><p class="hint">Actions 개발용 패키지 · 공개 배포 서명·공증과 실제 입력·설치 검증은 별도입니다. 비공개 저장소의 Actions 사용은 GitHub 요금제 한도를 따릅니다.</p><details class="card cross-platform"><summary>이 컴퓨터에서 빌드 · 프로젝트 내보내기</summary><p>${!ready ? "첫 스프라이트를 추가한 뒤 빌드하거나 내보낼 수 있습니다." : canBuildLocally ? `이 컴퓨터에서 ${platform}용 패키지를 만듭니다. 프로젝트를 GitHub에 업로드하지 않습니다.` : `이 프로젝트는 ${platform}을 빌드 대상으로 지정하지 않았습니다.`}</p><div class="row-actions"><button class="small" data-action="build" ${canBuildLocally ? "" : "disabled"}>${platform}용 빌드 ↗</button><button class="small" data-action="export-project" ${ready ? "" : "disabled"}>저장한 프로젝트 내보내기 ↓</button></div></details><div id="build-history">${buildHistory()}</div><details class="card"><summary>프로젝트 저장 기록</summary><p class="hint">저장할 때마다 구성을 기록합니다. 이전 구성과 리소스를 함께 복원할 수 있습니다.</p><button class="small" data-action="load-revisions">저장 기록 보기</button><div id="revision-history"></div></details>`;
}
function githubPanel() {
  return `<div class="card github-connection"><div class="card-heading"><h3>GitHub Actions</h3><span class="pill">${github.connected ? github.ready ? "빌드 준비 완료" : "워크플로 준비 필요" : "연결 안 됨"}</span></div>${github.connected ? `<p><strong>${esc(github.repository)}</strong> · ${github.private ? "비공개" : "공개"} · ${esc(github.login)}</p>${github.ready ? "" : `<p>기본 브랜치 ${esc(github.defaultBranch)}에 Studio 전용 워크플로 파일 하나를 ${github.workflowPresent ? "업데이트" : "추가"}합니다. 프로젝트는 빌드할 때 별도 브랜치에 업로드합니다.</p><button class="small" data-action="github-setup">워크플로 ${github.workflowPresent ? "업데이트" : "준비"}</button>`}` : `<p>Git 인증 또는 환경 변수의 GitHub 토큰으로 연결합니다. GitHub의 macOS와 Windows 빌드 환경을 사용합니다.</p>`}<p id="github-error" class="error-text" role="alert" ${githubError || github.error ? "" : "hidden"}>${esc(githubError || github.error)}</p><details ${github.connected ? "" : "open"}><summary>${github.connected ? "연결 변경" : "저장소와 인증"}</summary><label class="field"><span>GitHub 저장소</span><input id="github-repository" value="${esc(github.repository)}" placeholder="owner/repository" spellcheck="false" autocomplete="off"></label><label class="field"><span>토큰 (선택 · 기존 Git 인증을 쓰면 비워두세요)</span><input id="github-token" type="password" autocomplete="off" placeholder="이 Studio 실행 중에만 보관"></label><p class="hint">선택한 저장소의 Contents·Actions·Workflows 읽기/쓰기 권한이 필요합니다. 직접 입력한 토큰은 디스크·프로젝트·URL에 저장하지 않습니다. 연결 해제는 원격 빌드를 취소하지 않습니다.</p><div class="row-actions"><button class="small" data-action="github-connect">저장소 연결</button>${github.connected ? '<button class="small quiet" data-action="github-disconnect">연결 해제</button>' : ""}</div></details></div>`;
}
const removableBuild = new Set(["succeeded", "failed", "cancelled", "interrupted", "partial", "import_failed", "sync_failed"]);
const remoteActive = ["preparing", "queued", "running", "downloading", "awaiting_auth", "dispatch_unknown", "cancelling"];
const buildSteps: Record<string, string> = {
  "Set up job": "빌드 환경 준비",
  "Verify frozen engine and Studio request": "프로젝트와 엔진 확인",
  "Install pinned Rust toolchain": "Rust 도구 설치",
  "Install locked dependencies": "빌드 의존성 설치",
  "Check native builder contracts": "플랫폼 설정 검증",
  "Build independent app and integrity receipt": "앱과 설치 파일 생성",
  "Upload artifact": "빌드 결과 업로드",
};
function remoteJob(j: Job) {
  const states: Record<string, string> = { queued: "대기 중", waiting: "대기 중", pending: "대기 중", in_progress: "빌드 중", completed: "완료", success: "빌드 완료", failure: "실패", cancelled: "취소됨", skipped: "건너뜀", timed_out: "시간 초과", action_required: "GitHub 확인 필요" };
  const platforms = Object.entries(j.platforms ?? {}).map(([target, p]) => {
    const step = p.steps?.find((s) => s.status === "in_progress") ?? p.steps?.find((s) => s.conclusion === "failure");
    const status = p.imported ? "✓ 가져오기 완료" : !j.github?.runUrl && j.status === "cancelled" ? "취소됨" : states[p.conclusion || p.status] ?? p.status;
    return `<div><strong>${target === "macos-arm64" ? "macOS · Apple Silicon" : "Windows · x64"}</strong><span class="pill">${esc(status)}</span>${step ? `<p class="hint">${esc(buildSteps[step.name] ?? step.name)}</p>` : ""}${p.importError ? `<p class="error-text">${esc(p.importError)}</p>` : ""}</div>`;
  }).join("");
  const run = j.github?.runUrl ? `<a class="button small" target="_blank" rel="noreferrer" href="${esc(j.github.runUrl)}">Actions에서 상세 로그 ↗</a>` : "";
  const needsSync = j.status === "sync_failed" || (j.status === "failed" && !!j.github?.runId && !j.remoteConclusion);
  const needsImport = ["partial", "import_failed"].includes(j.status) || Object.values(j.platforms ?? {}).some(p => p.importError);
  const retry = !remoteActive.includes(j.status) && (needsSync || needsImport)
    ? `<button class="small" data-action="sync-build" data-id="${j.id}">${needsSync ? "Actions 다시 동기화" : "결과 다시 가져오기"}</button>` : "";
  return `<div class="remote-platforms">${platforms}</div>${run || retry ? `<div class="row-actions">${run}${retry}</div>` : ""}`;
}
function buildFiles(j: Job) {
  const downloads = j.downloads ?? [];
  const windowsPackage = (name: string) => name.endsWith("_windows-x64.zip");
  const hasWindowsPackage = downloads.some(d => windowsPackage(d.name));
  const installer = (name: string) =>
    /\.dmg$/i.test(name) || windowsPackage(name) || (!hasWindowsPackage && /\.(exe|msi)$/i.test(name));
  const guide = (name: string) => /^(?:Windows )?Installation Guide\.(pdf|html|png|jpe?g)$/.test(name);
  const link = (d: NonNullable<Job["downloads"]>[number]) => {
    const href = `/api/projects/${doc!.meta.id}/download/${j.id}/${encodeURIComponent(d.name)}`;
    if (installer(d.name)) {
      const label = windowsPackage(d.name) ? "Windows 설치 패키지 (ZIP)" : /\.dmg$/i.test(d.name) ? "macOS 설치 파일" : "Windows 설치 파일";
      return `<a class="button installer-link" href="${href}" download><strong>${label} ↓</strong><small>${esc(d.name)}${d.bytes ? ` · ${prettySize(d.bytes)}` : ""}</small></a>`;
    }
    const guideLabel = d.name.startsWith("Windows ") ? "Windows 설치 가이드" : "설치 가이드";
    return `<a class="button small" href="${href}" download>${guide(d.name) ? `${guideLabel} (${esc(d.name.split(".").at(-1)?.toUpperCase())})` : esc(d.name)} ↓</a>`;
  };
  const installers = downloads.filter(d => installer(d.name)), guides = downloads.filter(d => guide(d.name));
  const other = downloads.filter(d => !installer(d.name) && !guide(d.name));
  return `${installers.length ? `<div class="downloads installers">${installers.map(link).join("")}</div>` : ""}${guides.length ? `<div class="downloads">${guides.map(link).join("")}</div>` : ""}${other.length || j.result ? `<details data-build-detail="files"><summary>${other.length ? `패키지·진단 파일 ${other.length}개` : "결과 위치"}</summary>${j.result ? `<p class="artifact-path">${esc(j.result.directory)}</p>` : ""}<div class="downloads">${other.map(link).join("")}</div></details>` : ""}`;
}
function buildHistory() {
  const names: Record<string, string> = {
    queued: "대기 중",
    running: "빌드 중",
    cancelling: "취소 중",
    cancelled: "취소됨",
    succeeded: "완료",
    failed: "실패",
    interrupted: "중단됨",
    preparing: "업로드 준비 중", downloading: "결과 가져오는 중", awaiting_auth: "GitHub 재연결 필요",
    dispatch_unknown: "Actions 실행 확인 중", sync_failed: "Actions 조회 재시도 필요", partial: "일부 결과 완료", import_failed: "가져오기 재시도 필요",
  };
  return (
    jobs
      .map(
        (j) =>
          `<article class="build-card" data-job-id="${j.id}"><div class="card-heading"><div><span class="job-status ${j.status}">${names[j.status] ?? esc(j.status)}</span><strong>${date(j.createdAt)}</strong></div><div class="row-actions">${(j.provider === "github" ? remoteActive.filter(s => s !== "downloading" && s !== "cancelling") : ["queued", "running"]).includes(j.status) ? `<button class="small" data-action="cancel-build" data-id="${j.id}">빌드 취소</button>` : `<span class="hint">${j.development ? "개발용 빌드" : "릴리스 빌드"}</span>`}${removableBuild.has(j.status) ? `<button class="text-button danger" data-action="delete-build" data-id="${j.id}">기록 삭제</button>` : ""}</div></div><p class="build-revision">${j.provider === "github" ? "Actions / macOS + Windows" : j.target === "windows-x64" ? "Windows x64" : j.target === "macos-arm64" ? "macOS arm64" : "플랫폼 미기록"} · 구성 ${j.revision.slice(0, 8)} · ${j.id.slice(0, 8)}</p>${j.provider === "github" ? remoteJob(j) : ""}${j.error ? `<p class="error-text">${esc(j.error)}</p>` : ""}${buildFiles(j)}${j.logs ? `<details data-build-detail="logs"><summary>빌드 로그</summary><pre>${esc(j.logs)}</pre></details>` : ""}</article>`,
      )
      .join("") ||
    '<div class="empty-build"><span>▤</span><h3>아직 빌드한 앱이 없습니다.</h3><p>구성을 저장하고 첫 번째 패키지를 만들어보세요.</p></div>'
  );
}
function previewPanel() {
  if (!saved?.character.defaultAnimation) {
    const hasFirstSprite = !!doc!.character.defaultAnimation;
    return `<aside class="preview-panel"><div class="preview-heading"><h2>미리보기</h2></div><div class="preview-controls"><p class="hint">${hasFirstSprite ? "구성을 저장하면 가져온 스프라이트를 미리 볼 수 있습니다." : "첫 이미지나 GIF를 가져오면 여기에 버디가 표시됩니다."}</p>${hasFirstSprite ? '<button id="preview-save" class="primary small full-width" data-action="save">저장하고 미리보기</button>' : ""}</div></aside>`;
  }
  const selected = doc!.project.bindings[selectedBinding];
  const hasVariation = selected?.effect.type === "playAnimation" && selected.effect.variation;
  const variationControls = hasVariation ? '<p class="hint">이벤트를 다시 실행하면 다음 후보를 미리 봅니다.</p><button class="small" data-action="reset-motion-preview">모션 테스트 순서 초기화</button>' : "";
  return `<aside class="preview-panel"><div class="preview-heading"><h2>미리보기</h2><span class="preview-live"><i></i> PREVIEW</span></div><div class="preview-stage checker"><div class="preview-caption">YOUR LITTLE COMPANION</div><canvas id="preview-canvas" aria-label="버디 애니메이션 미리보기"></canvas><div class="preview-shadow"></div><span id="preview-state">idle</span></div><div class="preview-controls"><p id="preview-notice" class="hint"></p><button id="preview-save" class="primary small full-width" data-action="save" hidden>저장하고 미리보기</button><label class="field"><span>저장된 구성 · 애니메이션 재생</span><select id="preview-clip">${clips()
    .map((n) => option(n, n, selectedClip))
    .join(
      "",
    )}</select></label><button class="small full-width" data-action="replay">↻ 처음부터 재생</button><div class="preview-divider"></div><h3>이벤트 테스트</h3><p class="hint">저장된 구성으로 실제 동작 엔진을 실행합니다.</p><label class="field"><span>테스트할 이벤트</span><select id="test-binding">${doc!.project.bindings.map((b, i) => option(String(i), `${triggerNames[triggerCategory(b.trigger)]} · ${b.id}`, String(selectedBinding))).join("")}</select></label>${testInputSelector(selected?.trigger)}<button class="small full-width" data-action="test-event" ${doc!.project.bindings.length ? "" : "disabled"}>이벤트 실행 →</button>${variationControls}<div class="speed-test"><span>타이핑 속도</span><div><button data-action="test-speed" data-id="2">2 키/초</button><button data-action="test-speed" data-id="8">8 키/초</button></div><small>프로젝트의 측정 구간과 반응 타이밍으로 테스트합니다.</small></div><p class="preview-note">웹 미리보기는 선택한 테스트 입력만 사용합니다. 완성된 앱은 다른 앱에서 작업할 때도 반응합니다.</p><div class="preview-divider"></div><div class="project-facts"><span>리소스<strong>${resourceCount()}</strong></span><span>동작<strong>${clips().length}</strong></span><span>이벤트<strong>${doc!.project.bindings.length}</strong></span></div></div></aside>`;
}
function setPreviewWork(message = "") {
  previewWork = message;
  const panel = document.querySelector<HTMLElement>(".preview-panel");
  if (!panel) return;
  let status = panel.querySelector<HTMLElement>("#preview-work");
  if (!status) {
    status = document.createElement("p");
    status.id = "preview-work";
    status.className = "preview-work";
    status.setAttribute("role", "status");
    panel.querySelector(".preview-controls h3")?.after(status);
  }
  status.textContent = message;
  status.hidden = !message;
  for (const button of document.querySelectorAll<HTMLButtonElement>('[data-action="test-event"],[data-action="reset-motion-preview"],[data-action="test-speed"],[data-action="test-idle"],[data-action="test-cursor"]')) {
    if (doc?.meta.archived && button.closest(".editor-content")) continue;
    button.disabled = !!message
      || (button.dataset.action === "test-event" && !doc?.project.bindings.length)
      || (button.dataset.action === "test-cursor" && (!doc?.project.defaults.input.mouse || (!!idleGroupDraft && currentIdleGroup() === idleGroupDraft)));
  }
}
async function prepareSimulation(play: (request: number) => Promise<void>) {
  const request = ++simulationRequest;
  setPreviewWork("미리보기를 준비하는 중…");
  try {
    await play(request);
  } catch (error) {
    if (request === simulationRequest) throw error;
  } finally {
    if (request === simulationRequest) setPreviewWork();
  }
}
const previewAudio = new ReactionAudio((id) => {
  const sound = saved?.character.sounds[id];
  return sound ? resourceUrl(sound.audio.slice(10)) : undefined;
}, () => toast("미리보기에서 사운드를 재생하지 못했습니다.", true));
function savedFrameUrls(document: Document) {
  const base = `/api/projects/${document.meta.id}/frames/${document.meta.revision}`;
  return Object.fromEntries(document.images.map(name => [name, `${base}/${name}`]));
}
function sizePreview(
  preview: CharacterRenderer, character: Character, limit: number,
  extent = horizontalAnchorExtent(character),
) {
  const { frameWidth, frameHeight } = character.sheet;
  const width = frameWidth * 2 * extent;
  const fit = limit / Math.max(width, frameHeight);
  preview.resize(width * fit, frameHeight * fit);
}
function clipPreviewState(name: string): CharacterState {
  return {
    protocolVersion: 1,
    epoch: currentEpoch++,
    actionInstanceId: 1,
    animation: name,
    animationElapsedMs: 0,
    source: "idle",
    facingLeft: false,
    position: { x: 0, y: 0 },
    typingBandId: null,
  };
}
function updateScaleComparison() {
  if (!doc || !saved) return;
  const comparingIdle = selectedClip === doc.character.defaultAnimation;
  const extent = Math.max(horizontalAnchorExtent(saved.character), horizontalAnchorExtent(doc.character));
  scalePreviews.forEach((preview, index) => {
    const animations = index === 0 && comparingIdle
      ? saved!.character.animations
      : doc!.character.animations;
    sizePreview(preview, saved!.character, 180, extent);
    preview.setAnimationLayout(animations, animationScaleLimits.max / 100, extent);
  });
}
function previewClipFacing() {
  const preview = scalePreviews[1];
  const animation = doc?.character.animations[selectedClip];
  if (!preview || !animation) return;
  const facing = document.querySelector<HTMLSelectElement>("#clip-preview-facing")?.value;
  const state = clipPreviewState(selectedClip);
  state.facingLeft = !!facing && facing !== (animation.sourceFacing ?? "right");
  preview.update(state, 1);
}
function startScaleComparison() {
  if (!doc || !saved) return;
  const reference = document.querySelector<HTMLCanvasElement>("#clip-scale-reference");
  const selected = document.querySelector<HTMLCanvasElement>("#clip-scale-preview");
  if (!reference || !selected) return;
  const names = [doc.character.defaultAnimation, selectedClip];
  const imageUrls = savedFrameUrls(saved);
  scalePreviews = [reference, selected].map((canvas, index) => {
    const name = names[index];
    const clip = saved!.renderPlan.animations[name]!;
    const plan = { animations: { [name]: { ...clip, looping: true } } };
    return new CharacterRenderer(canvas, saved!.character, imageUrls, () => {}, plan);
  });
  updateScaleComparison();
  scalePreviews.forEach((preview, index) => preview.update(clipPreviewState(names[index]), 1));
}
function setClipScale(percent: number) {
  if (!doc || doc.meta.archived || !Number.isInteger(percent) ||
      percent < animationScaleLimits.min || percent > animationScaleLimits.max) return;
  const animation = doc.character.animations[selectedClip];
  for (const id of ["#clip-scale-range", "#clip-scale-number"]) {
    const input = document.querySelector<HTMLInputElement>(id)!;
    if (input.valueAsNumber !== percent) input.value = String(percent);
  }
  document.querySelector<HTMLButtonElement>("#reset-clip-scale")!.disabled = percent === 100;
  if ((animation.scalePercent ?? 100) === percent) return;
  if (percent === 100) delete animation.scalePercent;
  else animation.scalePercent = percent;
  markDirty();
  updateScaleComparison();
}
function startPreview() {
  if (!doc || !saved) return;
  const canvas = document.querySelector<HTMLCanvasElement>("#preview-canvas");
  if (!canvas) return;
  renderer = new CharacterRenderer(
    canvas,
    saved.character,
    savedFrameUrls(saved),
    () => {},
    saved.renderPlan,
  );
  previewDocument = saved;
  sizePreview(renderer, saved.character, 240);
  playClip(selectedClip);
}
function playClip(name: string) {
  simulationRequest++;
  setPreviewWork();
  if (!renderer || !saved) return;
  previewClip = name;
  simulationTimers.forEach(clearTimeout);
  simulationTimers = [];
  previewAudio.stop();
  renderer.update(clipPreviewState(name), 1);
  const label = document.querySelector("#preview-state");
  if (label)
    label.textContent = saved.character.animations[name]
      ? name
      : "저장 후 미리보기";
}
function updateProjectList(meta: Meta) {
  projects = [meta, ...projects.filter((project) => project.id !== meta.id)];
}
async function save(icon?: string) {
  if (!doc || busy || projectLoading || !dirty) return;
  setEditorBusy("편집 내용을 확인하는 중…");
  try {
    await flushDraft();
    setEditorBusy("구성과 리소스를 저장하고 미리보기에 반영하는 중…");
    const value = await projectApi("", {
      method: "PUT",
      body: JSON.stringify({ ...payload(), ...(icon ? { icon } : {}) }),
    });
    doc = value;
    saved = structuredClone(value);
    dirty = false;
    draftRevision = null;
    draftContent = "";
    draftState = "idle";
    restoredDraft = false;
    validationError = "";
    updateProjectList(value.meta);
    layout();
    toast("구성과 리소스를 새 버전으로 저장했습니다.");
  } catch (error) {
    showValidationError((error as Error).message);
    throw error;
  } finally {
    setEditorBusy();
  }
}
function setProjectLoading(loading: boolean) {
  projectLoading = loading;
  updateOperationStatus();
  const workspace = document.querySelector<HTMLElement>(".workspace");
  if (workspace) {
    workspace.inert = loading;
    if (loading) workspace.setAttribute("aria-busy", "true");
    else workspace.removeAttribute("aria-busy");
  }
}
async function openProject(id: string, route?: StudioRoute, push = !route) {
  const request = ++projectRequest;
  setProjectLoading(true);
  try {
    if (dirty) await flushDraft();
    if (request !== projectRequest) return;
    const [value, nextJobs] = await Promise.all([
      api<Document>(`projects/${id}`),
      api<Job[]>(`projects/${id}/builds`),
    ]);
    if (request !== projectRequest) return;
    saved = structuredClone(value);
    doc = value;
    jobs = nextJobs;
    selectedIdleGroup = "";
    selectedIdleDirection = "center";
    selectedIdlePhase = undefined;
    idleGroupDraft = undefined;
    dirty = false;
    restoredDraft = false;
    draftRevision = null;
    draftContent = "";
    draftState = "idle";
    validationError = "";
    if (
      value.draft?.baseRevision === value.meta.revision &&
      !value.meta.archived
    ) {
      doc.project = value.draft.project;
      doc.character = value.draft.character;
      doc.license = value.draft.license;
      dirty = true;
      restoredDraft = true;
      draftRevision = value.draft.draftRevision;
      draftState = "saved";
      draftContent = JSON.stringify(draftPayload());
    }
    selectedClip = route?.clip ?? "idle";
    selectedBinding = Math.max(
      0,
      doc.project.bindings.findIndex((b) => b.id === route?.bindingId),
    );
    if (route) tab = route.tab;
    archived = doc.meta.archived;
    layout(push);
  } catch (error) {
    if (request === projectRequest) throw error;
  } finally {
    if (request === projectRequest) setProjectLoading(false);
  }
}
async function ensureSaved() {
  if (dirty) await save();
}
async function uploadInstallAsset(slot: "background" | "guide", file: File) {
  if (!doc || busy || projectLoading || doc.meta.archived) return;
  if (slot === "background" && !/\.(png|jpe?g)$/i.test(file.name)) throw new Error("배경에는 PNG 또는 JPEG를 선택하세요.");
  if (file.size > 16 * 1024 * 1024) throw new Error("파일당 최대 크기는 16 MiB입니다.");
  setEditorBusy("설치 파일을 가져오는 중…");
  try {
    const resource = await projectApi<ResourceFile>(
      `resources?purpose=installer&filename=${encodeURIComponent(file.name)}`,
      { method: "POST", body: file },
    );
    doc.library[resource.name] = resource;
    (doc.project.build.macosDmg ??= defaultMacDmg())[slot] = `assets/installer/${resource.name}`;
    markDirty();
    layout();
  } finally { setEditorBusy(); }
}
async function upload(files: FileList | File[], icon = false) {
  if (!doc || busy || projectLoading) return;
  if (doc.meta.archived)
    throw new Error("보관된 프로젝트는 복원 후 수정하세요.");
  setEditorBusy("리소스 가져오기를 준비하는 중…");
  const hadIdle = !!doc.character.defaultAnimation;
  const errors: string[] = [],
    list = Array.from(files);
  let imported = 0,
    selectedIcon: string | undefined;
  try {
    for (const [index, file] of list.entries()) {
      setEditorBusy(`리소스를 가져오는 중 (${index + 1}/${list.length}) · ${file.name}`);
      try {
        if (file.size > 16 * 1024 * 1024)
          throw new Error("파일당 최대 크기는 16 MiB입니다.");
        const resource = await projectApi<ResourceFile>(
          `resources?filename=${encodeURIComponent(file.name)}`,
          { method: "POST", body: file },
        );
        doc.library[resource.name] = resource;
        if (icon) {
          selectedIcon = resource.name;
          markDirty();
        } else addResource(resource.name);
        imported++;
      } catch (error) {
        errors.push(`${file.name}: ${(error as Error).message}`);
      }
    }
  } finally {
    setEditorBusy();
    layout();
  }
  if (selectedIcon) await save(selectedIcon);
  if (errors.length)
    toast(
      `${imported}개 가져옴 · ${errors.length}개 실패\n${errors.join("\n")}`,
      true,
    );
  else if (!icon)
    toast(!hadIdle && doc.character.defaultAnimation
      ? "첫 이미지를 기본 대기 동작에 연결했습니다. 애니메이션에서 프레임을 고르거나 저장해 미리 보세요."
      : `${imported}개 리소스를 가져왔습니다. 동작에 연결한 뒤 저장하세요.`);
}
function addResource(filename: string) {
  if (isAudio(filename)) {
    const sounds = doc!.character.sounds;
    if (Object.values(sounds).some((sound) => sound.audio === `resources/${filename}`)) return;
    let n = 1;
    while (sounds[`sound-${n}`]) n++;
    sounds[`sound-${n}`] = { audio: `resources/${filename}` };
    markDirty();
    return;
  }
  doc!.character.resources ??= {};
  if (
    Object.values(doc!.character.resources).some(
      (r) => r.image === `resources/${filename}`,
    )
  )
    return;
  let n = 1;
  while (doc!.character.resources[`resource-${n}`]) n++;
  const id = `resource-${n}`;
  doc!.character.resources[id] = {
    image: `resources/${filename}`,
  };
  if (!doc!.character.defaultAnimation) {
    const animation = selectIdleAppearance(`resource:${id}`);
    doc!.character.defaultAnimation = animation;
    selectedClip = animation;
  }
  markDirty();
}
function chooseDefaultAnimation(loop: boolean) {
  return (
    clips().find((n) => doc!.character.animations[n].loop === loop) ?? ""
  );
}
function changeTrigger(type: string) {
  const b = doc!.project.bindings[selectedBinding],
    t: any = { type };
  if ((type === "typingStopped" || type === "buddyLongPress") && doc!.project.bindings.some(other => other !== b && other.trigger.type === type)) {
    layout();
    toast(`${triggerNames[type]} 이벤트는 프로젝트당 하나만 설정할 수 있습니다.`, true);
    return;
  }
  if (["keyDown", "keyUp", "whileKeyDown"].includes(type)) {
    const previous = b.trigger;
    t.code = "code" in previous ? previous.code : "Space";
    if (type === "whileKeyDown" && t.code === "*") t.code = "Space";
    if ("code" in previous && previous.modifiers != null) t.modifiers = previous.modifiers;
    if ("code" in previous && previous.alternatives?.length) {
      t.alternatives = previous.alternatives.map((key) => ({
        ...key,
        code: type === "whileKeyDown" && key.code === "*" ? "Space" : key.code,
      }));
    }
  }
  if (type === "keyDown") t.repeat = false;
  if (type === "typingRate") {
    t.minKps = 0.5;
    t.maxKps = 3;
  }
  if (type === "keySequence") {
    t.keyword = "안녕";
    t.layout = "ko-KR-2set";
    t.timeoutMs = 5000;
  }
  if (type === "idleFor") t.seconds = 300;
  if (type === "timerCompleted" || type === "timerStarted") t.phase = "countdown";
  if (type === "buddyLongPress") t.holdMs = buddyHoldMs.default;
  if (type === "typingStopped") {
    Object.assign(t, { minTypingSeconds: 8, minKeys: 12, pauseMs: 1500, minIntervalSeconds: 45 });
  }
  b.trigger = type === "mouseInput" ? { type: "mouseDown", button: "primary" } : t;
  b.effect =
    ["typingRate", "whileKeyDown", "drag"].includes(type)
        ? {
            type: "sustainAnimation",
            animation: doc!.character.animations[
              type === "drag" ? "walk" : "typing"
            ]?.loop
              ? type === "drag"
                ? "walk"
                : "typing"
              : chooseDefaultAnimation(true),
          }
        : b.effect.type === "playAnimation" && b.effect.variation
          ? { ...b.effect, stopOnRelease: supportsStopOnRelease(b.trigger) && b.effect.stopOnRelease }
          : {
              type: "playAnimation",
              animation: chooseDefaultAnimation(false),
              priority: 50,
              cooldownMs: 0,
              retrigger: "restart",
              stopOnRelease: false,
            };
  markDirty();
  layout();
}
function setupCrop() {
  const canvas = document.querySelector<HTMLCanvasElement>("#crop-canvas");
  if (!canvas) return;
  const a = doc!.character.animations[selectedClip],
    resource = a.resource ? doc!.character.resources[a.resource] : undefined;
  const sheet = doc!.character.image;
  if (!resource && !sheet) return;
  const image = new Image();
  cropImage = image;
  image.onload = () => {
    if (!canvas.isConnected || cropImage !== image) return;
    canvas.width = image.naturalWidth;
    canvas.height = image.naturalHeight;
    paintCrop();
  };
  cropImage.src = resource
    ? resourceUrl(resource.image.slice(10))
    : sourceUrl(sheet!);
  let start: { x: number; y: number } | null = null;
  const point = (e: PointerEvent) => {
    const box = canvas.getBoundingClientRect();
    return {
      x: Math.max(
        0,
        Math.min(
          canvas.width - 1,
          Math.floor(((e.clientX - box.left) * canvas.width) / box.width),
        ),
      ),
      y: Math.max(
        0,
        Math.min(
          canvas.height - 1,
          Math.floor(((e.clientY - box.top) * canvas.height) / box.height),
        ),
      ),
    };
  };
  canvas.onpointerdown = (e) => {
    start = point(e);
    canvas.setPointerCapture(e.pointerId);
  };
  canvas.onpointermove = (e) => {
    if (!start) return;
    const p = point(e);
    crop = {
      x: Math.min(p.x, start.x),
      y: Math.min(p.y, start.y),
      width: Math.max(1, Math.abs(p.x - start.x)),
      height: Math.max(1, Math.abs(p.y - start.y)),
    };
    for (const key of Object.keys(crop))
      (document.getElementById(`crop-${key}`) as HTMLInputElement).value =
        String(crop[key as keyof FrameRegion]);
    paintCrop();
  };
  canvas.onpointerup = () => {
    start = null;
  };
  canvas.onpointercancel = () => {
    start = null;
  };
}
function paintCrop() {
  const c = document.querySelector<HTMLCanvasElement>("#crop-canvas"),
    ctx = c?.getContext("2d");
  if (!ctx || !cropImage) return;
  ctx.clearRect(0, 0, c!.width, c!.height);
  ctx.drawImage(cropImage, 0, 0);
  ctx.fillStyle = "#064e3b22";
  ctx.fillRect(crop.x, crop.y, crop.width, crop.height);
  ctx.strokeStyle = "#09855c";
  ctx.lineWidth = Math.max(2, c!.width / 300);
  ctx.strokeRect(crop.x, crop.y, crop.width, crop.height);
}
async function simulate(
  events: { atMs: number; event: any }[],
  untilMs: number,
  request: number,
  { expectedBand, fromMs = 0, motionPreview, typingPreview }: {
    expectedBand?: string;
    fromMs?: number;
    motionPreview?: MotionPreview;
    typingPreview?: TypingPreview;
  } = {},
) {
  const projectId = doc!.meta.id;
  let timeline = await projectApi<SimulationFrame[]>(
    "simulate",
    post({ events, untilMs, fromMs, motionPreview, includeBindingIds: !!typingPreview }),
  );
  if (doc?.meta.id !== projectId || request !== simulationRequest) return;
  if (expectedBand && !timeline.some(({ state }) => state.typingBandId === expectedBand && state.source === "typingRate")) {
    const reached = timeline.some(({ state }) => state.typingBandId === expectedBand);
    throw new Error(reached
      ? "선택한 속도 구간에 도달했지만 다른 동작이 먼저 표시됩니다. 이벤트 우선순위를 확인하세요."
      : "테스트 입력이 선택한 속도 구간에 머물지 못했습니다. 입력 중단·속도 구간 안정 시간과 측정 구간을 확인하세요.");
  }
  if (typingPreview) {
    const started = timeline.find(frame => reactionStart(frame, typingPreview.bindingId));
    if (started?.atMs !== typingPreview.startAtMs)
      throw new Error(started ? "예상과 다른 시점에 타이핑 종료 반응이 시작됐습니다." : "예상 시각에 타이핑 종료 반응이 표시되지 않았습니다. 동시에 표시되는 동작을 확인하세요.");
  }
  const previewLimit = untilMs - fromMs;
  let previewEnd = previewLimit;
  if (motionPreview) {
    const matches = (frame: (typeof timeline)[number]) =>
      frame.bindingId === motionPreview.bindingId && frame.state.source === "oneShot";
    const first = timeline.findIndex(matches);
    if (first === -1) throw new Error("선택한 이벤트가 실행되지 않았습니다. 발생 조건과 우선순위를 확인하세요.");
    const instance = timeline[first].state.actionInstanceId;
    const second = timeline.findIndex((frame, index) => index > first && matches(frame) && frame.state.actionInstanceId !== instance);
    if (second !== -1) {
      previewEnd = timeline[second].atMs;
      timeline = timeline.slice(0, second);
    }
    motionPreview.starts++;
  }
  simulationTimers.forEach(clearTimeout);
  simulationTimers = [];
  const epoch = currentEpoch++;
  const last = timeline.at(-1);
  const restoreIdle = last?.state.source === "oneShot";
  let previous = "";
  for (const frame of timeline) {
    const key = `${frame.state.actionInstanceId}:${frame.state.animation}:${frame.state.facingLeft}`;
    if (key === previous) continue;
    previous = key;
    simulationTimers.push(
      window.setTimeout(() => {
        const state = { ...frame.state, epoch };
        renderer?.update(state, frame.atMs);
        previewAudio.update(state);
        document.querySelector("#preview-state")!.textContent =
          `${frame.state.animation} · ${frame.atMs}ms`;
      }, frame.atMs),
    );
  }
  if (restoreIdle) {
    simulationTimers.push(window.setTimeout(() => {
      const idle = saved!.character.defaultAnimation;
      previewAudio.stop();
      renderer?.update(clipPreviewState(idle), previewEnd);
      document.querySelector("#preview-state")!.textContent = idle + " · 미리보기 종료";
    }, previewEnd));
  }
  toast(typingPreview?.quietMessage ?? (restoreIdle && previewEnd === previewLimit
    ? "미리보기 제한까지 재생한 뒤 대기 상태로 돌아갑니다."
    : "실제 동작 엔진으로 이벤트를 재생합니다."));
}

function reactionStart(frame: SimulationFrame, bindingId: string) {
  return frame.bindingId === bindingId && frame.state.source === "oneShot" && frame.state.animationElapsedMs === 0;
}
function motionPreviewFor(binding: Binding): MotionPreview | undefined {
  if (binding.effect.type !== "playAnimation" || !binding.effect.variation || !saved) return;
  const { id, revision } = saved.meta;
  const key = `${id}:${revision}`;
  if (motionPreviewRevision !== key) {
    motionPreviewRevision = key;
    motionPreviews.clear();
  }
  let preview = motionPreviews.get(binding.id);
  if (!preview || preview.starts >= 10_000) {
    preview = { bindingId: binding.id, revision, seed: crypto.getRandomValues(new Uint32Array(1))[0], starts: 0 };
    motionPreviews.set(binding.id, preview);
  }
  return preview;
}
async function testBinding(index: number, variant: number, request: number) {
  const b = doc!.project.bindings[index];
  if (!b) return;
  const motionPreview = motionPreviewFor(b);
  const effect = b.effect;
  let previewTail = 2000;
  if (effect.type === "playAnimation") {
    const duration = effect.durationMs ?? Math.max(...motionClips(effect).map(name =>
      saved!.renderPlan.animations[name]?.frames.reduce((sum, frame) => sum + frame.durationMs, 0) ?? 0
    ));
    previewTail = Math.max(previewTail, Math.min(13000, 250 + Math.ceil(duration)));
  }
  if (b.effect.type === "move" && !doc!.project.defaults.movementEnabled)
    throw new Error("프로젝트 탭에서 키를 누른 채 이동을 먼저 켜세요.");
  if (
    ["keyDown", "keyUp", "whileKeyDown", "typingRate", "typingStopped", "keySequence"].includes(
      b.trigger.type,
    ) &&
    !doc!.project.defaults.input.keyboard
  )
    throw new Error("프로젝트 탭에서 키보드 감지를 먼저 켜주세요.");
  if (b.trigger.type === "timerCompleted" || b.trigger.type === "timerStarted") {
    if (!doc!.project.timer?.enabled) throw new Error("프로젝트 탭에서 타이머 사용을 먼저 켜세요.");
    const phase = timerTriggerPhases(b.trigger)[variant];
    if (!phase) throw new Error("테스트할 단계를 선택하세요.");
    return simulate([
      ...(b.trigger.type === "timerStarted" ? [{ atMs: 0, event: { type: "timerPhaseChanged", phase } }] : []),
      { atMs: 100, event: { type: b.trigger.type, phase } },
    ], 100 + previewTail, request, { motionPreview });
  }
  if (b.trigger.type === "buddyLongPress")
    return simulate([{ atMs: 100, event: { type: "buddyLongPress" } }], 100 + previewTail, request, { motionPreview });
  const pointer = pointerInputs(b.trigger);
  if (pointer) {
    const input = pointer[variant];
    if (!input) throw new Error("테스트할 마우스·트랙패드 입력을 선택하세요.");
    if (input.type !== "buddyClick" && !doc!.project.defaults.input.mouse)
      throw new Error("프로젝트 탭에서 마우스 감지를 먼저 켜주세요.");
    const events = input.type === "mouseDown"
      ? [
          { atMs: 100, event: input },
          { atMs: 800, event: { type: "mouseUp", button: input.button } },
        ]
      : [{ atMs: 100, event: input }];
    return simulate(events, previewTail + 100, request, { motionPreview });
  }
  const t: any = b.trigger,
    events: { atMs: number; event: any }[] = [];
  let end = previewTail, from = 0;
  const key = (code: string, atMs: number) => {
    events.push(
      { atMs, event: { type: "keyDown", code, repeat: false } },
      { atMs: atMs + 35, event: { type: "keyUp", code } },
    );
  };
  switch (t.type) {
    case "keyDown":
    case "keyUp":
    case "whileKeyDown": {
      const choice = variant === 0 ? t : t.alternatives?.[variant - 1];
      if (!choice) throw new Error("테스트할 키를 선택하세요.");
      for (const modifier of modifierOptions) {
        if (choice.modifiers?.includes(modifier.id)) {
          events.push({ atMs: 90, event: { type: "keyDown", code: modifier.key + "Left", repeat: false } });
        }
      }
      events.push(
        {
          atMs: 100,
          event: {
            type: "keyDown",
            code: choice.code === "*" ? "KeyA" : choice.code,
            repeat: false,
          },
        },
        {
          atMs: 800,
          event: { type: "keyUp", code: choice.code === "*" ? "KeyA" : choice.code },
        },
      );
      for (const modifier of modifierOptions) {
        if (choice.modifiers?.includes(modifier.id)) {
          events.push({ atMs: 810, event: { type: "keyUp", code: modifier.key + "Left" } });
        }
      }
      if (effect.type === "playAnimation") end += t.type === "keyUp" ? 800 : 100;
      break;
    }
    case "drag":
    case "dragDropped":
      events.push(
        { atMs: 100, event: { type: "dragStart" } },
        { atMs: 1200, event: { type: "dragEnd" } },
      );
      if (t.type === "dragDropped")
        events.push({ atMs: 1200, event: { type: "dragDropped" } });
      if (effect.type === "playAnimation") end += 1200;
      break;
    case "firstShown":
    case "shownAgain":
      if (t.type === "shownAgain")
        events.push({ atMs: 0, event: { type: "overlayVisibility", visible: true } });
      events.push(
        { atMs: 0, event: { type: "overlayVisibility", visible: false } },
        { atMs: 100, event: { type: "overlayVisibility", visible: true } },
      );
      end += 100;
      break;
    case "idleFor":
      from = t.seconds * 1000 - 100;
      end = from + 100 + previewTail;
      break;
    case "typingStopped":
      return testTypingStopped(b.id, b.trigger as TypingStoppedTrigger, variant, request, previewTail, motionPreview);
    case "typingRate": {
      const { minKps, maxKps } = b.trigger as Extract<Binding["trigger"], { type: "typingRate" }>;
      const windowMs = doc!.project.defaults.timing.rateWindowMs;
      // The runtime measures an integer key count over the full window.
      let keys = Math.floor(minKps * windowMs / 1000);
      if (keys * 1000 / windowMs < minKps) keys++;
      const rate = keys * 1000 / windowMs;
      if (rate >= (maxKps ?? Infinity))
        throw new Error("현재 측정 구간으로는 이 범위의 속도가 나오지 않습니다. 속도 범위를 넓히거나 측정 구간을 조정하세요.");
      if (keys > 250)
        throw new Error("이 속도 구간을 테스트하려면 250회 넘게 키를 눌러야 합니다. 설치한 앱에서 테스트하세요.");
      return testSpeed(rate, request, b.id);
    }
    case "keySequence": {
      const savedIndex = saved!.project.bindings.findIndex((binding) => binding.id === b.id);
      const steps = variant === 0
        ? saved!.sequences?.[savedIndex]
        : saved!.sequenceAlternatives?.[savedIndex]?.[variant - 1];
      if (!steps) throw new Error("키워드 입력을 선택하세요.");
      let at = 100;
      for (const step of steps) {
        if (step.shift)
          events.push({
            atMs: at - 1,
            event: { type: "keyDown", code: "ShiftLeft", repeat: false },
          });
        key(step.code, at);
        if (step.shift)
          events.push({
            atMs: at + 36,
            event: { type: "keyUp", code: "ShiftLeft" },
          });
        at += 80;
      }
      events.sort((a, b) => a.atMs - b.atMs);
      end = Math.min(15000, at + Math.max(2500, previewTail));
      break;
    }
  }
  await simulate(events, end, request, { fromMs: from, motionPreview });
}

async function testTypingStopped(
  bindingId: string, trigger: TypingStoppedTrigger, variant: number, request: number,
  previewTail: number, motionPreview?: MotionPreview,
) {
  const { minTypingSeconds, minKeys, pauseMs } = trigger;
  const events: { atMs: number; event: { type: "keyDown" | "keyUp"; code: string } }[] = [];
  const tap = (atMs: number, index = 0) => {
    const code = ["KeyJ", "KeyK", "KeyL"][index % 3];
    events.push({ atMs, event: { type: "keyDown", code } }, { atMs: atMs + 5, event: { type: "keyUp", code } });
  };
  const burst = (start: number) => {
    const span = minTypingSeconds * 1000;
    const count = Math.max(minKeys, Math.ceil(span / (pauseMs - 10)) + 1);
    for (let index = 0; index < count; index++) tap(start + Math.round(index * span / (count - 1)), index);
    return start + span + pauseMs;
  };
  if (variant === 1) {
    tap(100);
    const from = pauseMs;
    return simulate(events, from + 400, request, {
      fromMs: from,
      typingPreview: { bindingId, quietMessage: "짧은 입력에서는 타이핑 종료 반응이 시작되지 않았습니다." },
    });
  }
  const first = burst(100);
  if (variant === 3) {
    if (!typingRepeatFits(trigger)) throw new Error("두 번째 입력 구간이 끝나면 재반응 간격이 이미 지납니다.");
    const projectId = doc!.meta.id;
    const initial = await projectApi<SimulationFrame[]>(
      "simulate", post({ events, fromMs: first, untilMs: first, includeBindingIds: true }),
    );
    if (doc?.meta.id !== projectId || request !== simulationRequest) return;
    if (!initial.some(frame => reactionStart(frame, bindingId)))
      throw new Error("첫 번째 입력 구간에서 반응이 시작되지 않았습니다. 동시에 표시되는 동작을 확인하세요.");
    const second = burst(first + 1);
    const from = second - 100;
    return simulate(events, second + 300, request, {
      fromMs: from,
      typingPreview: { bindingId, quietMessage: "최소 재반응 간격 안에는 두 번째 반응이 시작되지 않았습니다." },
    });
  }
  if (variant === 2) tap(first);
  const expected = variant === 2 ? first + pauseMs : first;
  const from = first - 100;
  return simulate(events, Math.min(from + 15000, expected + previewTail), request, {
    fromMs: from, motionPreview,
    typingPreview: { bindingId, startAtMs: expected - from },
  });
}
async function testSpeed(rate: number, request: number, expectedBand?: string) {
  if (!doc!.project.defaults.input.keyboard)
    throw new Error("프로젝트 탭에서 키보드 감지를 먼저 켜세요.");
  const timing = doc!.project.defaults.timing;
  const interval = 1000 / rate;
  const inputUntil = Math.max(2000, timing.rateWindowMs + timing.rateEvaluationMs + timing.rateStabilityMs + 400);
  const events = [];
  let last = 0;
  for (let index = 0; index < 250 && index * interval < inputUntil; index++) {
    last = Math.round(index * interval);
    events.push(
      { atMs: last, event: { type: "keyDown", code: "KeyA", repeat: false } },
      { atMs: last + Math.min(20, Math.floor(interval / 2)), event: { type: "keyUp", code: "KeyA" } },
    );
  }
  await simulate(events, Math.min(15000, last + timing.idleTimeoutMs + 250), request, { expectedBand });
}
function confirmDraftChange(title: string, message: string, label: string) {
  const dialog = document.createElement("dialog");
  dialog.setAttribute("aria-labelledby", "draft-confirmation-title");
  dialog.innerHTML = `<h2 id="draft-confirmation-title">${esc(title)}</h2><p>${esc(message)}</p><footer><button type="button" autofocus>취소</button><button type="button" class="primary">${esc(label)}</button></footer>`;
  const [cancel, accept] = dialog.querySelectorAll("button");
  cancel.onclick = () => dialog.close();
  accept.onclick = () => dialog.close("confirmed");
  document.body.append(dialog);
  return new Promise<boolean>((resolve) => {
    dialog.addEventListener("close", () => {
      dialog.remove();
      resolve(dialog.returnValue === "confirmed");
    }, { once: true });
    dialog.showModal();
  });
}
async function renameResource(action: "rename-resource" | "rename-sound", id: string) {
  const project = doc;
  if (!project || project.meta.archived) return;
  const resource = action === "rename-resource"
    ? project.character.resources[id] : project.character.sounds[id];
  if (!resource) return;
  const original = originalResourceName(id, resource);
  const dialog = document.createElement("dialog");
  dialog.setAttribute("aria-labelledby", "resource-name-title");
  dialog.innerHTML = `<form method="dialog"><h2 id="resource-name-title">리소스 이름 수정</h2>
    <p>원본 파일 · ${esc(original)}</p>
    <label class="field"><span>표시 이름</span><input name="resourceName" maxlength="80" value="${esc(resource.name)}" placeholder="${esc(original)}" autofocus></label>
    <p class="hint">비워 두면 원본 이름을 사용합니다.</p>
    <footer><button type="button">취소</button><button class="primary" value="save">적용</button></footer></form>`;
  const input = dialog.querySelector("input")!;
  dialog.querySelector<HTMLButtonElement>('button[type="button"]')!.onclick = () => dialog.close();
  document.body.append(dialog);
  const name = await new Promise<string | null>((resolve) => {
    dialog.addEventListener("close", () => {
      dialog.remove();
      resolve(dialog.returnValue === "save" ? input.value.trim() : null);
    }, { once: true });
    dialog.showModal();
    input.select();
  });
  if (name === null || doc !== project || name === (resource.name ?? "")) return;
  if (name) resource.name = name;
  else delete resource.name;
  markDirty();
  layout();
  app.querySelector<HTMLElement>(`[data-action="${action}"][data-id="${CSS.escape(id)}"]`)?.focus();
}
function chooseBuildRemoval(job: Job) {
  const dialog = document.createElement("dialog");
  dialog.setAttribute("aria-labelledby", "build-removal-title");
  dialog.innerHTML = `<h2 id="build-removal-title">빌드 기록을 삭제할까요?</h2>
    <p>${date(job.createdAt)} 빌드가 목록에서 사라집니다. 파일도 함께 삭제하면 이 Studio가 보관한 설치 파일과 결과물이 제거됩니다. 이미 내려받은 파일과 GitHub Actions 실행은 유지됩니다.</p>
    <footer class="row-actions"><button type="button" value="cancel" autofocus>취소</button><button type="button" value="record">기록만 삭제</button><button type="button" value="files" class="danger">파일도 삭제</button></footer>`;
  document.body.append(dialog);
  return new Promise<string>((resolve) => {
    const finish = (choice: string) => {
      dialog.close();
      dialog.remove();
      resolve(choice);
    };
    for (const button of dialog.querySelectorAll("button"))
      button.onclick = () => finish(button.value);
    dialog.addEventListener("cancel", (event) => {
      event.preventDefault();
      finish("cancel");
    });
    dialog.showModal();
  });
}
async function action(name: string, id?: string) {
  if (busy) return;
  if (projectLoading && !["open", "export-draft", "retry-connection"].includes(name)) return;
  if (
    ["clone", "archive", "restore", "discard-draft", "cancel-build", "delete-build"].includes(
      name,
    )
  ) {
    if (["clone", "archive"].includes(name) && dirty) await save();
    const messages: Record<string, string> = {
      clone: "프로젝트를 복제하는 중…",
      archive: "보관 상태를 바꾸는 중…",
      restore: "이전 구성 복원을 준비하는 중…",
      "discard-draft": "변경 취소를 준비하는 중…",
      "cancel-build": "빌드 취소를 요청하는 중…",
      "delete-build": "빌드 기록을 정리하는 중…",
    };
    setEditorBusy(messages[name]);
    try {
      return await performAction(name, id);
    } finally {
      setEditorBusy();
    }
  }
  return performAction(name, id);
}
async function performAction(name: string, id?: string) {
  switch (name) {
    case "github-connect":
    case "github-setup":
    case "github-disconnect": {
      const repository = document.querySelector<HTMLInputElement>("#github-repository")?.value ?? github.repository;
      const input = document.querySelector<HTMLInputElement>("#github-token"), token = input?.value;
      if (input) input.value = "";
      setEditorBusy("GitHub 연결 설정을 적용하는 중…"); githubError = "";
      try {
        github = await api<GitHubState>(`github/${name.slice(7)}`, post({ repository, ...(token ? { token } : {}) }));
        layout(); toast(name === "github-setup" ? "두 플랫폼 빌드 워크플로를 준비했습니다." : name === "github-connect" ? "GitHub 저장소를 연결했습니다." : "GitHub 연결을 해제했습니다.");
      } catch (e) {
        githubError = (e as Error).message;
        const error = document.querySelector<HTMLElement>("#github-error");
        if (error) { error.textContent = githubError; error.hidden = false; }
        throw e;
      } finally { setEditorBusy(); }
      break;
    }
    case "sync-build":
      await projectApi(`builds/${id}/sync`, post({}));
      jobs = await projectApi("builds"); layout();
      toast("기존 Actions 실행의 상태와 결과를 다시 확인합니다.");
      break;
    case "build-remote": {
      await ensureSaved();
      const projectId = doc!.meta.id;
      const job = await projectApi<Job>("builds", post({ revision: doc!.meta.revision, provider: "github", repository: github.repository, acknowledgeUpload: true }));
      const next = await api<Job[]>(`projects/${projectId}/builds`);
      if (doc?.meta.id !== projectId) return;
      jobs = next; tab = "builds"; layout();
      toast(`두 플랫폼 빌드 ${job.id.slice(0,8)}를 준비합니다. 편집을 계속해도 빌드 구성은 고정됩니다.`);
      break;
    }
    case "export-project": {
      await ensureSaved();
      setEditorBusy("프로젝트 파일을 내보내는 중…");
      try {
        const response = await fetch(`/api/projects/${doc!.meta.id}/export/${doc!.meta.revision}`);
        if (!response.ok) throw new Error((await response.json()).error ?? "내보내기에 실패했습니다.");
        const url = URL.createObjectURL(await response.blob());
        const link = document.createElement("a");
        link.href = url;
        link.download = `${doc!.meta.id}-${doc!.meta.revision.slice(0, 8)}.tar.gz`;
        link.click();
        window.setTimeout(() => URL.revokeObjectURL(url), 1000);
        toast("저장한 구성과 원본 리소스를 내보냈습니다.");
      } finally {
        setEditorBusy();
      }
      break;
    }
    case "export-draft": {
      const url = URL.createObjectURL(
        new Blob([JSON.stringify(draftPayload(), null, 2)], {
          type: "application/json",
        }),
      );
      const link = document.createElement("a");
      link.href = url;
      link.download = `${doc!.meta.id}.draft.json`;
      link.click();
      window.setTimeout(() => URL.revokeObjectURL(url), 1000);
      break;
    }
    case "discard-draft": {
      if (!(await confirmDraftChange(
        "저장한 구성으로 돌아갈까요?",
        "마지막으로 임시 저장한 내용은 프로젝트의 drafts 폴더에 보관합니다.",
        "저장한 구성으로 돌아가기",
      )))
        return;
      clearTimeout(draftTimer);
      await draftChain.catch(() => {});
      await projectApi("draft", {
        method: "DELETE",
        body: JSON.stringify({ draftRevision }),
      });
      dirty = false;
      draftRevision = null;
      draftContent = "";
      draftState = "idle";
      restoredDraft = false;
      validationError = "";
      await openProject(doc!.meta.id);
      toast("마지막 저장 구성으로 돌아왔습니다.");
      break;
    }
    case "retry-connection":
      clearTimeout(pollTimer);
      await poll();
      break;
    case "new":
      (
        document.querySelector("#project-dialog") as HTMLDialogElement
      ).showModal();
      break;
    case "close-dialog":
      (document.querySelector("#project-dialog") as HTMLDialogElement).close();
      break;
    case "open":
      await openProject(id!);
      break;
    case "archive-list":
      archived = !archived;
      layout();
      break;
    case "tab":
      tab = id!;
      layout(true);
      break;
    case "save":
      await save();
      break;
    case "validate":
      setEditorBusy("리소스와 이벤트 연결을 검증하는 중…");
      try {
        await projectApi("validate", post(payload()));
        validationError = "";
        document.querySelector("#validation-panel")?.remove();
      } catch (error) {
        showValidationError((error as Error).message);
        throw error;
      } finally {
        setEditorBusy();
      }
      toast(doc!.character.defaultAnimation
        ? "구성, 리소스와 이벤트 연결을 검증했습니다."
        : "프로젝트 설정을 검증했습니다. 첫 스프라이트를 추가하면 미리 보고 빌드할 수 있습니다.");
      break;
    case "configure-follow":
      doc!.project.cursorFollow = { enabled: false, stopDistanceDip: 48, bands: [{ minDistanceDip: 0, speedDipPerSecond: 120 }] };
      markDirty();
      layout();
      document.querySelector("#follow-settings")?.scrollIntoView({ block: "nearest" });
      break;
    case "configure-dmg":
      doc!.project.build.macosDmg = defaultMacDmg();
      markDirty();
      layout();
      break;
    case "remove-dmg":
      delete doc!.project.build.macosDmg;
      markDirty();
      layout();
      break;
    case "restore-install-asset":
    case "remove-install-asset": {
      if (id !== "background" && id !== "guide") return;
      const dmg = doc!.project.build.macosDmg ??= defaultMacDmg();
      delete dmg[id];
      dmg[defaultDmgField(id)] = name === "restore-install-asset";
      if (id === "guide" && !dmg.useDefaultGuide) delete dmg.guidePosition;
      markDirty();
      layout();
      break;
    }
    case "remove-follow":
      delete doc!.project.cursorFollow;
      markDirty();
      layout();
      break;
    case "add-follow-band": {
      const bands = doc!.project.cursorFollow!.bands;
      const previous = bands.at(-1)!;
      if (bands.length >= 8 || previous.minDistanceDip >= 10000) throw new Error("더 높은 거리 구간을 추가할 수 없습니다.");
      bands.push({ minDistanceDip: Math.min(10000, previous.minDistanceDip + 180), speedDipPerSecond: Math.min(1000, previous.speedDipPerSecond + 100) });
      markDirty();
      layout();
      break;
    }
    case "remove-follow-band":
      if (Number(id) > 0) doc!.project.cursorFollow!.bands.splice(Number(id), 1);
      markDirty();
      layout();
      break;
    case "clone":
      await ensureSaved();
      const copy = await projectApi<Document>("clone", post({}));
      projects = await api("projects");
      await openProject(copy.meta.id);
      toast("새 앱 식별자로 프로젝트를 복제했습니다.");
      break;
    case "archive":
      if (dirty) {
        await save();
      }
      await projectApi("archive", post({ archived: !doc!.meta.archived }));
      projects = await api("projects");
      doc = await projectApi();
      saved = structuredClone(doc);
      archived = doc!.meta.archived;
      layout();
      break;
    case "clip":
    case "reference-clip":
      selectedClip = id!;
      if (name === "reference-clip") tab = "animations";
      layout(true);
      break;
    case "binding":
    case "reference-binding":
      selectedBinding = Number(id);
      if (name === "reference-binding") tab = "events";
      layout(true);
      break;
    case "replay":
      playClip(
        (document.querySelector("#preview-clip") as HTMLSelectElement).value,
      );
      break;
    case "reset-clip-scale":
      setClipScale(100);
      break;
    case "add-clip": {
      const n = prompt("새 동작 이름 (영문, 숫자, -, _)", "new-animation");
      if (!n) return;
      if (!/^[a-zA-Z0-9_-]{1,64}$/.test(n) || doc!.character.animations[n])
        throw new Error("겹치지 않는 영문 동작 이름을 입력하세요.");
      const character = doc!.character;
      const resource = character.image ? null
        : character.animations[selectedClip]?.resource ?? Object.keys(character.resources)[0];
      if (!character.image && !resource) throw new Error("리소스에서 이미지를 먼저 가져오세요.");
      character.animations[n] = {
        frames: resource ? [] : [0],
        fps: 12,
        loop: false,
        resource,
        regions: [],
        useSourceTiming: true,
      };
      selectedClip = n;
      markDirty();
      layout();
      break;
    }
    case "remove-clip":
      if (usedByIdle(selectedClip))
        throw new Error("이 동작을 사용하는 대기 상태를 먼저 수정하세요.");
      if (doc!.project.cursorFollow?.bands.some((band) => band.animation === selectedClip))
        throw new Error("이 동작을 사용하는 마우스 따라오기 구간을 먼저 수정하세요.");
      if (
        doc!.project.bindings.some(
          (b) => usesAnimation(b.effect, selectedClip),
        )
      )
        throw new Error("이 동작을 사용하는 이벤트를 먼저 수정하세요.");
      delete doc!.character.animations[selectedClip];
      selectedClip = doc!.character.defaultAnimation;
      markDirty();
      layout();
      break;
    case "rename-resource":
    case "rename-sound":
      if (id) await renameResource(name, id);
      break;
    case "remove-sound":
      if (doc!.project.bindings.some((binding) => binding.sound?.resource === id))
        throw new Error("이 사운드를 사용하는 이벤트를 먼저 수정하세요.");
      delete doc!.character.sounds[id!];
      markDirty();
      layout();
      break;
    case "remove-base-sprite":
      if (Object.values(doc!.character.animations).some(animation => !animation.resource))
        throw new Error("기본 스프라이트를 사용하는 동작을 먼저 변경하세요.");
      delete doc!.character.image;
      markDirty();
      layout();
      break;
    case "remove-resource":
      if (
        Object.values(doc!.character.animations).some((a) => a.resource === id)
      )
        throw new Error("이 리소스를 사용하는 애니메이션을 먼저 수정하세요.");
      delete doc!.character.resources[id!];
      markDirty();
      layout();
      break;
    case "reuse-resource":
      addResource(id!);
      layout();
      break;
    case "append-gif-frame":
    case "clear-gif-frames": {
      const animation = doc!.character.animations[selectedClip];
      if (name === "append-gif-frame") {
        const frame = document.querySelector<HTMLInputElement>("#gif-frame-number")!;
        if (frame.value === "" || !frame.reportValidity()) return;
        if (animation.frames.length >= 256) throw new Error("재생 목록은 최대 256프레임입니다.");
        animation.frames.push(frame.valueAsNumber);
      } else {
        animation.frames = [];
      }
      document.querySelector<HTMLInputElement>("#clip-frames")!.value = animation.frames.join(", ");
      markDirty();
      break;
    }
    case "clear-regions": {
      const a = doc!.character.animations[selectedClip];
      a.regions = [];
      a.frames = a.resource ? [] : [0];
      markDirty();
      layout();
      break;
    }
    case "apply-region":
    case "append-region": {
      const a = doc!.character.animations[selectedClip];
      if (name === "apply-region") a.regions = [];
      a.regions ??= [];
      a.regions.push({ ...crop });
      a.frames = [];
      markDirty();
      layout();
      break;
    }
    case "remove-region":
      doc!.character.animations[selectedClip].regions.splice(Number(id), 1);
      markDirty();
      layout();
      break;
    case "make-grid": {
      const value = (id: string) =>
          Number((document.getElementById(id) as HTMLInputElement).value),
        w = value("grid-width"),
        h = value("grid-height"),
        start = value("grid-start"),
        count = value("grid-count"),
        cols = Math.floor((cropImage?.naturalWidth ?? 0) / w),
        rows = Math.floor((cropImage?.naturalHeight ?? 0) / h);
      if (
        ![w, h, start, count].every(Number.isInteger) ||
        w < 1 ||
        h < 1 ||
        start < 0 ||
        count < 1 ||
        count > 256 ||
        start + count > cols * rows
      )
        throw new Error(
          "이미지 안에 들어가는 셀 크기와 프레임 범위를 지정하세요.",
        );
      const a = doc!.character.animations[selectedClip];
      a.regions = Array.from({ length: count }, (_, i) => ({
        x: ((start + i) % cols) * w,
        y: Math.floor((start + i) / cols) * h,
        width: w,
        height: h,
      }));
      a.frames = [];
      markDirty();
      layout();
      break;
    }
    case "add-binding": {
      let n = 1;
      while (doc!.project.bindings.some((b) => b.id === `event-${n}`)) n++;
      doc!.project.bindings.push({
        id: `event-${n}`,
        trigger: { type: "keyDown", code: "Enter", repeat: false },
        effect: {
          type: "playAnimation",
          animation: chooseDefaultAnimation(false),
          priority: 50,
          retrigger: "restart",
          cooldownMs: 0,
          durationMs: null,
          stopOnRelease: false,
        },
      });
      selectedBinding = doc!.project.bindings.length - 1;
      markDirty();
      layout();
      break;
    }
    case "add-motion":
    case "remove-motion":
    case "move-motion": {
      const effect = doc!.project.bindings[selectedBinding]?.effect;
      if (effect?.type !== "playAnimation") break;
      const candidates = motionCandidates(effect);
      if (name === "add-motion") {
        const next = clips().find(clip => !doc!.character.animations[clip].loop && !candidates.some(candidate => candidate.animation === clip));
        if (!next || candidates.length >= 8) break;
        candidates.push({ animation: next, weight: 1 });
      } else {
        const [index, direction] = (id ?? "").split(":").map(Number);
        if (!Number.isInteger(index) || index < 0 || index >= candidates.length || candidates.length < 2) break;
        if (name === "remove-motion") candidates.splice(index, 1);
        else {
          const other = index + direction;
          if ((direction !== -1 && direction !== 1) || other < 0 || other >= candidates.length) break;
          [candidates[index], candidates[other]] = [candidates[other], candidates[index]];
        }
      }
      setMotionCandidates(effect, candidates);
      break;
    }
    case "timer-preset-add":
    case "timer-preset-move":
    case "timer-preset-remove": {
      const timer = doc!.project.timer;
      if (!timer?.enabled) break;
      const presets = [...(timer.countdownPresets ?? defaultTimerPresets)];
      if (name === "timer-preset-add") {
        if (presets.length >= timerPresetLimit) break;
        const seconds = [timer.countdownSeconds, ...Array.from({ length: timerPresetLimit }, (_, i) => (i + 1) * 60)]
          .find(seconds => seconds > 0 && !presets.includes(seconds));
        if (seconds === undefined) break;
        presets.push(seconds);
      } else {
        const [index, direction] = (id ?? "").split(":").map(Number);
        if (!Number.isInteger(index) || index < 0 || index >= presets.length) break;
        if (name === "timer-preset-remove") presets.splice(index, 1);
        else {
          const other = index + direction;
          if ((direction !== -1 && direction !== 1) || other < 0 || other >= presets.length) break;
          [presets[index], presets[other]] = [presets[other], presets[index]];
        }
      }
      timer.countdownPresets = presets;
      markDirty();
      layout();
      break;
    }
    case "enable-project-timer":
      setTimerEnabled(true);
      break;
    case "reset-motion-preview":
      motionPreviews.delete(doc!.project.bindings[selectedBinding].id);
      toast("모션 테스트 순서를 초기화했습니다.");
      break;
    case "capture-key": {
      const button = captureButton(Number(id));
      if (button) setKeyCapture(button, button.getAttribute("aria-pressed") !== "true");
      break;
    }
    case "add-key-choice": {
      const trigger = doc!.project.bindings[selectedBinding].trigger;
      if (!("code" in trigger)) break;
      const alternatives = trigger.alternatives ??= [];
      if (alternatives.length >= 31) break;
      alternatives.push({ code: "", modifiers: [] });
      markDirty();
      layout();
      const button = captureButton(alternatives.length);
      if (button) setKeyCapture(button, true);
      break;
    }
    case "remove-key-choice": {
      const trigger = doc!.project.bindings[selectedBinding].trigger;
      if (!("code" in trigger) || !trigger.alternatives?.length) break;
      const index = Number(id);
      if (index === 0) {
        const [next] = trigger.alternatives.splice(0, 1);
        trigger.code = next.code;
        trigger.modifiers = next.modifiers;
      } else {
        trigger.alternatives.splice(index - 1, 1);
      }
      markDirty();
      layout();
      break;
    }
    case "add-pointer-input": {
      const binding = doc!.project.bindings[selectedBinding];
      const inputs = pointerInputs(binding.trigger);
      if (!inputs) break;
      const used = new Set(inputs.map(pointerInputKey));
      const available = pointerOptions.filter(({ input }) => !used.has(pointerInputKey(input)));
      const next = available.find(({ input }) => input.type === inputs.at(-1)?.type) ?? available[0];
      if (!next) break;
      setPointerInputs(binding, [...inputs, next.input]);
      document.querySelector<HTMLElement>(`[data-pointer-input="${inputs.length}"]`)?.focus();
      break;
    }
    case "remove-pointer-input": {
      const binding = doc!.project.bindings[selectedBinding];
      const inputs = pointerInputs(binding.trigger);
      if (!inputs || inputs.length < 2) break;
      setPointerInputs(binding, inputs.filter((_, index) => index !== Number(id)));
      break;
    }
    case "add-keyword-choice": {
      const trigger = doc!.project.bindings[selectedBinding].trigger;
      if (trigger.type !== "keySequence") break;
      const alternatives = trigger.alternatives ??= [];
      if (alternatives.length >= 31) break;
      alternatives.push({ keyword: "", layout: trigger.layout === "en-US-qwerty" ? "ko-KR-2set" : "en-US-qwerty" });
      markDirty();
      layout();
      document.querySelector<HTMLInputElement>(
        `[data-path="project.bindings.${selectedBinding}.trigger.alternatives.${alternatives.length - 1}.keyword"]`,
      )?.focus();
      break;
    }
    case "remove-keyword-choice": {
      const trigger = doc!.project.bindings[selectedBinding].trigger;
      if (trigger.type !== "keySequence" || !trigger.alternatives?.length) break;
      const index = Number(id);
      if (index === 0) {
        const [next] = trigger.alternatives.splice(0, 1);
        trigger.keyword = next.keyword;
        trigger.layout = next.layout;
        delete trigger.steps;
      } else {
        trigger.alternatives.splice(index - 1, 1);
      }
      markDirty();
      layout();
      break;
    }
    case "select-idle-group":
      selectedIdleGroup = id ?? "";
      selectedIdlePhase = undefined;
      layout();
      app.querySelector<HTMLElement>('.idle-group-list [aria-pressed="true"]')?.focus();
      break;
    case "add-idle-group": {
      if (!idleGroupDraft) {
        if (timerPhases.every(phase => idleStateOwner(phase))) break;
        const groups = doc!.project.idleGroups ?? [];
        let index = 1;
        while (groups.some(group => group.id === `idle-group-${index}`)) index++;
        idleGroupDraft = { id: `idle-group-${index}`, states: [], cursorMode: "inherit" };
      }
      selectedIdleGroup = idleGroupDraft.id;
      selectedIdlePhase = undefined;
      layout();
      app.querySelector<HTMLElement>("[data-idle-state]:enabled")?.focus();
      break;
    }
    case "commit-idle-group": {
      const group = idleGroupDraft;
      if (!group?.states.length || group.states.some(state => idleStateOwner(state) !== group)) break;
      const next = structuredClone(group);
      const original = doc!.character.animations;
      doc!.character.animations = { ...original };
      try {
        const resolve = (animation: string | null | undefined) => animation?.startsWith("resource:") ? selectIdleAppearance(animation) : animation;
        next.animation = resolve(next.animation);
        for (const pose of Object.values(next.directions ?? {})) pose.animation = resolve(pose.animation);
      } catch (error) {
        doc!.character.animations = original;
        throw error;
      }
      (doc!.project.idleGroups ??= []).push(next);
      idleGroupDraft = undefined;
      markDirty();
      layout();
      app.querySelector<HTMLElement>('.idle-group-list [aria-pressed="true"]')?.focus();
      break;
    }
    case "discard-idle-group":
      idleGroupDraft = undefined;
      selectedIdleGroup = "";
      layout();
      app.querySelector<HTMLElement>('[data-action="add-idle-group"]')?.focus();
      break;
    case "remove-idle-group":
      doc!.project.idleGroups = doc!.project.idleGroups?.filter(group => group.id !== selectedIdleGroup);
      selectedIdleGroup = "";
      markDirty();
      layout();
      break;
    case "idle-direction":
      if (cursorDirections.some(([direction]) => direction === id)) {
        selectedIdleDirection = id as CursorDirection;
        layout();
        app.querySelector<HTMLElement>('.idle-direction-picker [aria-pressed="true"]')?.focus();
      }
      break;
    case "test-idle":
    case "test-cursor": {
      const group = currentIdleGroup();
      if (group && group === idleGroupDraft) return;
      const phase = selectedIdlePhase && group?.states.includes(selectedIdlePhase) ? selectedIdlePhase : group?.states[0];
      if (phase && !doc!.project.timer?.enabled) throw new Error("프로젝트 탭에서 타이머 사용을 먼저 켜세요.");
      if (name === "test-cursor" && !doc!.project.defaults.input.mouse)
        throw new Error("프로젝트 탭에서 마우스 감지를 먼저 켜주세요.");
      await ensureSaved();
      const events = phase ? [{ atMs: 0, event: { type: "timerPhaseChanged", phase } }] : [];
      const index = cursorDirections.findIndex(([direction]) => direction === id);
      const positions = [120, 228, 330];
      await prepareSimulation((request) => simulate(index < 0 ? events : [...events,
        { atMs: 100, event: { type: "cursorMove", x: positions[index % 3], y: positions[Math.floor(index / 3)] } },
      ], index < 0 ? 0 : 1500, request));
      break;
    }
    case "reorder-binding": {
      const destination = selectedBinding + Number(id);
      const bindings = doc!.project.bindings;
      if (destination < 0 || destination >= bindings.length) return;
      [bindings[selectedBinding], bindings[destination]] = [bindings[destination], bindings[selectedBinding]];
      selectedBinding = destination;
      markDirty();
      layout(true);
      break;
    }
    case "remove-binding":
      doc!.project.bindings.splice(selectedBinding, 1);
      markDirty();
      layout();
      break;
    case "timing-preset":
      doc!.project.defaults.timing = {
        rateWindowMs: 2000,
        rateEvaluationMs: 50,
        rateStabilityMs: id === "snappy" ? 50 : id === "relaxed" ? 200 : 100,
        idleTimeoutMs: id === "snappy" ? 150 : id === "relaxed" ? 500 : 250,
      };
      markDirty();
      layout();
      break;
    case "test-event": {
      const index = Number(document.querySelector<HTMLSelectElement>("#test-binding")?.value);
      const variant = Number(document.querySelector<HTMLSelectElement>("#test-variant")?.value ?? 0);
      await ensureSaved();
      await prepareSimulation((request) => testBinding(index, variant, request));
      break;
    }
    case "test-speed":
      await ensureSaved();
      await prepareSimulation((request) => testSpeed(Number(id), request));
      break;
    case "build": {
      await ensureSaved();
      const projectId = doc!.meta.id,
        previous = new Set(jobs.map((j) => j.id));
      const job = await projectApi<Job>(
        "builds",
        post({ revision: doc!.meta.revision, development: config.development }),
      );
      const result = await api<Job[]>(`projects/${projectId}/builds`);
      if (doc?.meta.id !== projectId) return;
      jobs = result;
      tab = "builds";
      layout();
      toast(
        previous.has(job.id)
          ? "진행 중인 빌드로 이동했습니다."
          : "빌드를 시작했습니다. 진행 중에도 다른 프로젝트를 편집할 수 있습니다.",
      );
      break;
    }
    case "cancel-build":
      await projectApi(`builds/${id}/cancel`, post({}));
      jobs = await projectApi("builds");
      layout();
      break;
    case "delete-build": {
      const job = jobs.find(job => job.id === id);
      if (!job || !removableBuild.has(job.status)) return;
      const removal = await chooseBuildRemoval(job);
      if (removal !== "record" && removal !== "files") return;
      const result = await projectApi<{ builtSlug?: string }>(
        `builds/${job.id}${removal === "files" ? "?files=true" : ""}`,
        { method: "DELETE" },
      );
      if (result.builtSlug) doc!.meta.builtSlug = result.builtSlug;
      jobs = await projectApi("builds");
      layout();
      toast(removal === "files" ? "빌드 기록과 파일을 삭제했습니다." : "빌드 기록을 삭제했습니다.");
      break;
    }
    case "load-revisions": {
      const history = await projectApi<
        {
          revision: string;
          name: string;
          identifier: string;
          version: string;
          updatedAt: string;
        }[]
      >("revisions");
      document.querySelector("#revision-history")!.innerHTML = history
        .map((r) => {
          const current = r.revision === doc!.meta.revision;
          const previousPackage = r.identifier !== doc!.meta.identifier;
          const label = current ? "현재 버전" : previousPackage ? "이전 패키지" : "복원";
          return `<div class="library-row"><span>${date(r.updatedAt)} · ${esc(r.name)} v${esc(r.version)}</span><button class="text-button" data-action="restore" data-id="${r.revision}" ${current || previousPackage ? "disabled" : ""}>${label}</button></div>`;
        })
        .join("");
      break;
    }
    case "restore": {
      if (
        dirty &&
        !(await confirmDraftChange(
          "이전 구성을 복원할까요?",
          "현재 임시 편집은 프로젝트의 drafts 폴더에 보관합니다.",
          "이전 구성 복원",
        ))
      )
        return;
      await flushDraft();
      const restored = await projectApi<Document>(
        "restore",
        post({ revision: id, expectedRevision: doc!.meta.revision, draftRevision }),
      );
      doc = restored;
      saved = structuredClone(restored);
      dirty = false;
      draftRevision = null;
      draftContent = "";
      draftState = "idle";
      restoredDraft = false;
      validationError = "";
      updateProjectList(restored.meta);
      layout();
      toast("선택한 구성과 리소스를 복원했습니다.");
      break;
    }
  }
}
app.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLElement>(
    "[data-action]",
  );
  if (button && !button.hasAttribute("disabled"))
    void action(button.dataset.action!, button.dataset.id).catch((e) =>
      toast(e.message, true),
    );
});
function activeKeyCapture(target: EventTarget | null) {
  return target instanceof HTMLButtonElement && target.dataset.action === "capture-key" &&
    target.getAttribute("aria-pressed") === "true" ? target : null;
}
app.addEventListener("keydown", (event) => {
  const button = activeKeyCapture(event.target);
  if (!button) return;
  event.preventDefault();
  event.stopPropagation();
  if (event.repeat) return;
  const status = button.closest(".key-capture")!.querySelector<HTMLElement>("[data-key-status]")!;
  if (!physicalKeys.has(event.code)) {
    pendingModifier = undefined;
    status.textContent = "지원되는 키를 다시 눌러주세요.";
    return;
  }
  const modifiers = modifierOptions
    .filter(({ key, flag }) => event[flag] && !event.code.startsWith(key))
    .map(({ id }) => id);
  if (modifierOptions.some(({ key }) => event.code.startsWith(key))) {
    pendingModifier = { code: event.code, modifiers };
    status.textContent = "조합할 키를 누르거나, 보조 키를 떼서 그 키만 등록하세요.";
    return;
  }
  saveCapturedKey(button, event.code, modifiers);
});
app.addEventListener("keyup", (event) => {
  const button = activeKeyCapture(event.target);
  if (button && pendingModifier?.code === event.code) {
    event.preventDefault();
    saveCapturedKey(button, pendingModifier.code, pendingModifier.modifiers);
  }
});
app.addEventListener("focusout", (event) => {
  const button = activeKeyCapture(event.target);
  if (button) setKeyCapture(button, false);
  const input = event.target;
  if (input instanceof HTMLInputElement && input.id === "clip-scale-number" &&
      (!input.value || !input.validity.valid)) {
    input.value = String(doc?.character.animations[selectedClip]?.scalePercent ?? 100);
  }
  if (input instanceof HTMLInputElement && input.id === "clip-anchor-x" && !input.validity.valid) {
    input.value = anchorPercent(doc?.character.animations[selectedClip]?.anchorX);
  }
});
window.addEventListener("blur", () => {
  const button = activeKeyCapture(document.activeElement);
  if (button) setKeyCapture(button, false);
});
app.addEventListener("input", (event) => {
  const input = event.target as HTMLInputElement;
  if (input.dataset.motionWeight !== undefined) {
    const effect = doc?.project.bindings[selectedBinding]?.effect;
    if (doc?.meta.archived || effect?.type !== "playAnimation" || effect.variation?.mode !== "weighted") return;
    const weights = motionCandidates(effect).map(candidate => candidate.weight);
    const index = Number(input.dataset.motionWeight);
    if (!Number.isInteger(index) || index < 0 || index >= weights.length) return;
    weights[index] = input.validity.badInput ? -1 : Number(input.value);
    effect.variation.weights = weights;
    markDirty();
    const total = motionWeightTotal(weights);
    app.querySelectorAll("[data-motion-probability]").forEach((label, index) => {
      label.textContent = motionProbability(weights[index], total);
    });
    app.querySelector<HTMLElement>("#motion-weight-warning")!.hidden = total > 0;
    return;
  }
  if (input.hasAttribute("data-timer-time") && doc?.project.timer) {
    const seconds = readTimerDuration([...app.querySelectorAll<HTMLInputElement>("[data-timer-time]")]);
    setPath("project.timer.countdownSeconds", seconds);
    document.querySelector("#timer-time-hint")!.textContent = seconds
      ? "처음 직접 입력할 때 채워지는 값입니다. 1초부터 3시간까지 설정할 수 있습니다."
      : "시간·분·초를 확인하세요. 합계는 1초부터 3시간까지입니다.";
    return;
  }
  if (input.hasAttribute("data-timer-preset-unit") && doc?.project.timer) {
    const row = input.closest<HTMLElement>("[data-timer-preset]")!;
    const presets = [...(doc.project.timer.countdownPresets ?? defaultTimerPresets)];
    presets[Number(row.dataset.timerPreset)] = readTimerDuration([...row.querySelectorAll<HTMLInputElement>("[data-timer-preset-unit]")]);
    setPath("project.timer.countdownPresets", presets);
    document.querySelector("#timer-preset-warning")!.textContent = timerPresetWarning(presets);
    return;
  }
  if (input.id === "clip-scale-number" || input.id === "clip-scale-range") {
    if (input.validity.valid) setClipScale(input.valueAsNumber);
    return;
  }
  if (input.id === "clip-anchor-x") {
    const animation = doc?.character.animations[selectedClip];
    if (!animation || doc?.meta.archived || !input.validity.valid) return;
    const anchor = input.value === "" ? undefined : input.valueAsNumber / 100;
    if ((animation.anchorX ?? undefined) === anchor) return;
    if (anchor === undefined) delete animation.anchorX;
    else animation.anchorX = anchor;
    markDirty();
    updateScaleComparison();
    return;
  }
  if (input.form?.id === "new-project-form") {
    const field = input.form.elements.namedItem(
      "packageId",
    ) as HTMLInputElement;
    if (input.name === "packageId") field.dataset.edited = "true";
    if (input.name === "name" && !field.dataset.edited) {
      let part =
        input.value
          .toLowerCase()
          .replace(/[^a-z0-9]/g, "")
          .slice(0, 48) || "buddy";
      if (!/^[a-z]/.test(part)) part = "buddy" + part;
      field.value = "io.desktopbuddy.app." + part;
    }
  }
  if (input.dataset.path) {
    setPath(
      input.dataset.path,
      input.type === "checkbox"
        ? input.checked
        : input.type === "number"
          ? Number(input.value)
          : input.value,
    );
    return;
  }
  if (input.id.startsWith("crop-")) {
    const key = input.id.slice(5) as keyof FrameRegion;
    if (key in crop) {
      crop[key] = Number(input.value);
      paintCrop();
    }
  }
  if (input.id === "gif-frame-number") {
    if (input.value !== "" && input.validity.valid) showGifFrame(input.valueAsNumber);
    else {
      clearTimeout(gifFrameTimer);
      gifFrameRequest++;
      document.querySelector("#gif-frame-status")!.textContent = "범위 안의 프레임 번호를 입력하세요.";
    }
  }
  if (input.id === "event-volume" && input.value !== "" && input.validity.valid) {
    const cue = doc!.project.bindings[selectedBinding].sound;
    if (cue) {
      cue.volume = input.valueAsNumber / 100;
      markDirty();
    }
  }
  if (input.id === "clip-frames") {
    const values = input.value
      .split(",")
      .map((v) => v.trim())
      .filter(Boolean)
      .map(Number);
    doc!.character.animations[selectedClip].frames = values;
    doc!.character.animations[selectedClip].regions = [];
    markDirty();
  }
  if (input.id === "max-kps") {
    (doc!.project.bindings[selectedBinding].trigger as any).maxKps = input.value
      ? Number(input.value)
      : null;
    markDirty();
  }
  if (input.id === "duration-ms") {
    (doc!.project.bindings[selectedBinding].effect as any).durationMs =
      input.value ? Number(input.value) : null;
    markDirty();
  }
});
app.addEventListener("change", (event) => {
  const input = event.target as HTMLInputElement;
  if (input.dataset.timerPhase !== undefined) {
    const trigger = doc?.project.bindings[selectedBinding]?.trigger;
    const phase = input.dataset.timerPhase as TimerPhase;
    if (doc?.meta.archived || (trigger?.type !== "timerCompleted" && trigger?.type !== "timerStarted") || !Object.hasOwn(timerPhaseLabels, phase)) return;
    const previous = timerTriggerPhases(trigger);
    const selected = previous.filter(candidate => candidate !== phase);
    if (input.checked) selected.push(phase);
    if (!selected.length) return;
    const [first, ...alternatives] = selected;
    trigger.phase = first;
    trigger.alternatives = alternatives;
    markDirty();
    layout();
    app.querySelector<HTMLElement>(`[data-timer-phase="${phase}"]:enabled`)?.focus();
    return;
  }
  if (input.id === "buddy-hold-seconds") {
    const trigger = doc?.project.bindings[selectedBinding]?.trigger;
    if (trigger?.type !== "buddyLongPress" || doc?.meta.archived) return;
    if (!input.validity.valid || !Number.isFinite(input.valueAsNumber)) {
      input.value = String(trigger.holdMs / 1000);
      toast("누르기 시간을 0.3~2초로 입력하세요.", true);
      return;
    }
    trigger.holdMs = Math.round(input.valueAsNumber * 1000);
    markDirty();
    return;
  }
  if (input.id === "idle-preview-phase") {
    selectedIdlePhase = input.value as TimerPhase;
    return;
  }
  if (input.dataset.idleState || input.id === "idle-group-name" || input.id === "idle-group-cursor-mode") {
    const group = currentIdleGroup();
    if (!group || doc?.meta.archived) return;
    if (input.dataset.idleState) {
      const phase = input.dataset.idleState as TimerPhase;
      const owner = idleStateOwner(phase);
      if (!timerPhases.includes(phase) || (owner && owner !== group)) return;
      const next = timerPhases.filter(state => state === phase ? input.checked : group.states.includes(state));
      if (!next.length && group !== idleGroupDraft) return;
      group.states = next;
    } else if (input.id === "idle-group-name") {
      group.name = input.value.trim() || undefined;
      const name = idleGroupName(group);
      app.querySelector("#idle-group-heading")!.textContent = name;
      app.querySelector('.idle-group-list [aria-pressed="true"] strong')!.textContent = name;
      if (group !== idleGroupDraft) markDirty();
      return;
    } else {
      group.cursorMode = input.value as IdleCursorMode;
    }
    if (group !== idleGroupDraft) markDirty();
    layout();
    if (input.dataset.idleState)
      app.querySelector<HTMLElement>(`[data-idle-state="${input.dataset.idleState}"]:enabled`)?.focus();
    return;
  }
  if (input.id === "timer-enabled" && doc && !doc.meta.archived) {
    setTimerEnabled(input.checked);
    return;
  }
  if (input.dataset.path?.endsWith(".effect.animation") && app.querySelector('[data-action="add-motion"]:disabled')) {
    layout();
  }
  if (input.id === "motion-mode" || input.dataset.motionIndex !== undefined) {
    const effect = doc?.project.bindings[selectedBinding]?.effect;
    if (doc?.meta.archived || effect?.type !== "playAnimation" || !effect.variation) return;
    if (input.id === "motion-mode") {
      if (input.value === "weighted") {
        effect.variation.weights = motionCandidates(effect).map(candidate => candidate.weight);
      }
      effect.variation.mode = input.value === "weighted" || input.value === "shuffle" ? input.value : "cycle";
      markDirty();
      layout();
      app.querySelector<HTMLElement>("#motion-mode")?.focus();
    } else {
      const candidates = motionCandidates(effect);
      const index = Number(input.dataset.motionIndex);
      if (Number.isInteger(index) && index >= 0 && index < candidates.length) {
        candidates[index].animation = input.value;
        setMotionCandidates(effect, candidates);
      }
    }
    return;
  }
  if (input.id === "clip-source-facing") {
    const animation = doc?.character.animations[selectedClip];
    if (!animation || doc?.meta.archived) return;
    if (input.value === "left") animation.sourceFacing = "left";
    else delete animation.sourceFacing;
    markDirty();
    previewClipFacing();
    return;
  }
  if (input.id === "clip-preview-facing") {
    previewClipFacing();
    return;
  }
  if (input.id === "dmg-guide-position-mode") {
    const dmg = doc!.project.build.macosDmg!;
    if (input.value === "manual") dmg.guidePosition = initialGuidePosition(dmg);
    else delete dmg.guidePosition;
    markDirty();
    layout();
    document.querySelector<HTMLElement>("#dmg-guide-position-mode")?.focus();
    return;
  }
  if (input.id === "display-size-mode") {
    const defaults = doc!.project.defaults;
    if (input.value === "height") {
      const current = (doc!.idleHeightDipAtScaleOne ?? doc!.character.sheet.frameHeight) * defaults.scale;
      defaults.displayHeightDip = Math.max(16, Math.min(1024, Math.round(current * 10) / 10));
    } else {
      delete defaults.displayHeightDip;
    }
    markDirty();
    layout();
    document.querySelector<HTMLElement>("#display-size-mode")?.focus();
    return;
  }
  const slot = input.dataset.installAsset;
  if (slot === "background" || slot === "guide") {
    const file = input.files?.[0];
    if (file) void uploadInstallAsset(slot, file).catch(error => toast(error.message, true));
    return;
  }
  if (input.dataset.followBand !== undefined) {
    doc!.project.cursorFollow!.bands[Number(input.dataset.followBand)].animation = input.value || undefined;
    markDirty();
    return;
  }
  if (input.dataset.pointerInput !== undefined) {
    const index = Number(input.dataset.pointerInput);
    const binding = doc!.project.bindings[selectedBinding];
    const inputs = pointerInputs(binding.trigger);
    const choice = pointerOptions.find(({ input: option }) => pointerInputKey(option) === input.value);
    if (inputs?.[index] && choice) {
      setPointerInputs(binding, inputs.map((value, i) => i === index ? choice.input : value));
      document.querySelector<HTMLElement>(`[data-pointer-input="${index}"]`)?.focus();
    }
    return;
  }
  if (input.hasAttribute("data-key-modifier-mode") || input.dataset.keyModifier ||
      (input.dataset.path?.endsWith(".code") && input.closest("[data-key-index]"))) {
    const trigger = doc!.project.bindings[selectedBinding].trigger;
    if (!("code" in trigger)) return;
    const index = Number(input.closest<HTMLElement>("[data-key-index]")?.dataset.keyIndex);
    const chord = index === 0 ? trigger : trigger.alternatives?.[index - 1];
    if (!chord) return;
    if (input.hasAttribute("data-key-modifier-mode")) {
      chord.modifiers = input.value === "exact" ? [] : undefined;
    } else if (chord.modifiers != null) {
      const selected = chord.modifiers;
      chord.modifiers = modifierOptions
        .filter(({ id, key }) => !chord.code.startsWith(key) &&
          (input.dataset.keyModifier === id ? input.checked : selected.includes(id)))
        .map(({ id }) => id);
    }
    markDirty();
    layout();
    const details = document.querySelector<HTMLDetailsElement>(`[data-key-index="${index}"] details`);
    if (details) details.open = true;
    return;
  }
  if (input.id === "resource-upload" && input.files)
    void upload(input.files).catch((e) => toast(e.message, true));
  if (input.id === "icon-upload" && input.files)
    void upload(input.files, true).catch((e) => toast(e.message, true));
  if (input.id === "clip-resource") {
    const a = doc!.character.animations[selectedClip];
    a.resource = input.value === "sheet" ? null : input.value;
    a.frames = a.resource ? [] : [0];
    a.regions = [];
    markDirty();
    layout();
  }
  if (input.id === "event-sound") {
    const binding = doc!.project.bindings[selectedBinding];
    binding.sound = input.value ? { resource: input.value, volume: binding.sound?.volume ?? 0.7 } : null;
    markDirty();
    layout();
  }
  if (input.id === "interruption-policy") {
    const effect = doc!.project.bindings[selectedBinding].effect;
    if (effect.type !== "playAnimation") return;
    effect.interruption = input.value as Interruption;
    markDirty();
    layout();
  }
  if (input.id === "continuous-priority") {
    const effect = doc!.project.bindings[selectedBinding].effect;
    if (effect.type === "move" || effect.type === "sustainAnimation") {
      effect.priority = input.value === "" ? null : Number(input.value);
      markDirty();
    }
  }
  if (input.id === "follow-priority" && input.validity.valid && doc?.project.cursorFollow) {
    if (input.value === "") delete doc.project.cursorFollow.priority;
    else doc.project.cursorFollow.priority = input.valueAsNumber;
    markDirty();
  }
  if (input.id === "idle-cursor-enabled") {
    const idle = doc!.project.idle ??= { cursorEnabled: false, directions: {} };
    idle.cursorEnabled = input.checked;
    markDirty();
    layout();
  }
  if (input.id === "idle-appearance" || input.dataset.idlePoseField) {
    try {
      const group = currentIdleGroup();
      const appearance = input.value
        ? (group && group === idleGroupDraft && input.value.startsWith("resource:") ? input.value : undefined)
        : null;
      if (input.id === "idle-appearance") {
        if (group) group.animation = appearance === undefined ? selectIdleAppearance(input.value) : appearance;
        else doc!.character.defaultAnimation = selectIdleAppearance(input.value);
      } else {
        const direction = selectedIdleDirection;
        const directions = group ? group.directions ??= {} : doc!.project.idle!.directions;
        if (group && group.cursorMode !== "custom") return;
        const pose = { ...directions[direction] };
        if (input.dataset.idlePoseField === "animation") {
          pose.animation = appearance === undefined ? selectIdleAppearance(input.value) : appearance;
        } else {
          pose.mirror = input.value === "" ? null : input.value === "true";
        }
        if (pose.animation == null && pose.mirror == null) delete directions[direction];
        else directions[direction] = pose;
      }
      if (!group || group !== idleGroupDraft) markDirty();
      layout();
    } catch (error) {
      layout();
      toast((error as Error).message, true);
    }
  }
  if (input.id === "trigger-type") changeTrigger(input.value);
  if (input.id === "held-effect") {
    doc!.project.bindings[selectedBinding].effect =
      input.value === "move"
        ? {
            type: "move",
            x: 1,
            y: 0,
            speedDipPerSecond: 120,
            animation: clips().find(
              (n) => n !== "idle" && doc!.character.animations[n].loop,
            ),
          }
        : { type: "sustainAnimation", animation: chooseDefaultAnimation(true) };
    markDirty();
    layout();
  }
  if (input.id === "preview-clip") {
    selectedClip = input.value;
    playClip(input.value);
    syncRoute(true);
    if (tab === "animations") layout();
  }
  if (input.id === "test-binding") {
    selectedBinding = Number(input.value); syncRoute(true);
    layout();
  }
});
app.addEventListener("dragover", (event) => {
  if ((event.target as HTMLElement).closest("#dropzone"))
    event.preventDefault();
});
app.addEventListener("drop", (event) => {
  if ((event.target as HTMLElement).closest("#dropzone")) {
    event.preventDefault();
    if (event.dataTransfer?.files)
      void upload(event.dataTransfer.files).catch((e) =>
        toast(e.message, true),
      );
  }
});
app.addEventListener("submit", (event) => {
  if ((event.target as HTMLElement).id !== "new-project-form") return;
  event.preventDefault();
  if (busy || projectLoading) return;
  const element = event.target as HTMLFormElement,
    form = new FormData(element),
    error = element.querySelector<HTMLElement>("#new-project-error")!;
  void (async () => {
    setEditorBusy("새 프로젝트를 만드는 중…");
    error.hidden = true;
    try {
      await flushDraft();
      const value = await api<Document>(
        "projects",
        post({
          name: form.get("name"),
          packageId: form.get("packageId"),
          template: form.get("template"),
        }),
      );
      projects = await api("projects");
      archived = false;
      tab = value.character.defaultAnimation ? "overview" : "resources";
      await openProject(value.meta.id);
      toast("새 프로젝트를 만들었습니다.");
    } catch (cause) {
      error.textContent = (cause as Error).message;
      error.hidden = false;
    } finally {
      setEditorBusy();
    }
  })();
});
window.addEventListener("popstate", () => {
  if (busy) { syncRoute(); return; }
  const route = readRoute(new URL(location.href));
  void (async () => {
    if (route.projectId && route.projectId !== doc?.meta.id) await openProject(route.projectId, route, false);
    else if (doc) {
      projectRequest++;
      setProjectLoading(false);
      tab = route.tab; selectedClip = route.clip;
      selectedBinding = Math.max(0, doc.project.bindings.findIndex((b) => b.id === route.bindingId));
      layout();
    }
  })().catch((error) => { syncRoute(); toast(error.message, true); });
});
window.addEventListener("beforeunload", (event) => {
  if (dirty && draftState !== "saved") {
    event.preventDefault();
    event.returnValue = "";
  }
});
window.addEventListener("keydown", (event) => {
  if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s") {
    event.preventDefault();
    if (!document.querySelector("dialog[open]"))
      void save().catch((error) => toast(error.message, true));
  }
});
async function poll() {
  if (polling) return;
  polling = true;
  clearTimeout(pollTimer);
  try {
    const nextConnection = await api<GitHubState>("github");
    if (JSON.stringify(nextConnection) !== JSON.stringify(github)) {
      github = nextConnection;
      const panel = document.querySelector("#github-panel");
      if (panel) panel.innerHTML = githubPanel();
      for (const button of document.querySelectorAll<HTMLButtonElement>('[data-action="build-remote"]'))
        button.disabled = !doc?.character.defaultAnimation || doc.meta.archived || !hasBothBuildTargets(doc.project.build.targets) || !(github.connected && github.ready);
    }
    if (doc) {
      const id = doc.meta.id,
        next = await projectApi<Job[]>("builds");
      if (doc?.meta.id === id) {
        const changed = JSON.stringify(jobs) !== JSON.stringify(next);
        jobs = next;
        const el = document.querySelector("#build-history");
        if (el && changed) {
          const stateKey = (details: HTMLDetailsElement) => `${details.closest<HTMLElement>("[data-job-id]")?.dataset.jobId}:${details.dataset.buildDetail}`;
          const states = new Map([...el.querySelectorAll<HTMLDetailsElement>("details[data-build-detail]")].map((details) => {
            const pre = details.querySelector("pre");
            return [stateKey(details), { open: details.open, top: pre?.scrollTop ?? 0, bottom: !!pre && pre.scrollHeight - pre.clientHeight - pre.scrollTop < 3 }] as const;
          }));
          const focused = document.activeElement as HTMLElement | null;
          const focusKey = (node: HTMLElement) => [node.closest<HTMLElement>("[data-job-id]")?.dataset.jobId, node.dataset.action, node.getAttribute("href"), node.closest<HTMLDetailsElement>("[data-build-detail]")?.dataset.buildDetail].join(":");
          const focus = focused && el.contains(focused) ? focusKey(focused) : null;
          el.innerHTML = buildHistory();
          for (const details of el.querySelectorAll<HTMLDetailsElement>("details[data-build-detail]")) {
            const state = states.get(stateKey(details));
            if (!state) continue;
            details.open = state.open;
            const pre = details.querySelector("pre");
            if (pre) pre.scrollTop = state.bottom ? pre.scrollHeight : state.top;
          }
          if (focus) [...el.querySelectorAll<HTMLElement>("a,button,summary")].find((node) => focusKey(node) === focus)?.focus({ preventScroll: true });
        }
      }
    }
  } catch {
    // api() keeps a persistent connection banner; retry without discarding edits.
  } finally { polling = false; }
  pollTimer = window.setTimeout(poll, 2000);
}
async function start() {
  const route = readRoute(new URL(location.href));
  const token = location.hash.slice(1);
  if (token) {
    history.replaceState(null, "", location.pathname + location.search);
    await api("session", {
      method: "POST",
      headers: { Authorization: `Bearer ${token}` },
    });
  }
  config = await api("config");
  github = await api("github");
  projects = await api("projects");
  layout();
  const first =
    projects.find(
      (p) =>
        p.id === route.projectId ||
        p.legacyIds?.includes(route.projectId ?? ""),
    ) ?? projects.find((p) => !p.archived);
  if (first) await openProject(first.id, route);
  void poll();
}
start().catch((error) => {
  app.innerHTML = `<div class="connection-error"><h1>Studio에 연결하지 못했습니다.</h1><p>${esc(error.message)}</p><p>터미널에 표시된 Studio 주소를 다시 여세요.</p></div>`;
});
