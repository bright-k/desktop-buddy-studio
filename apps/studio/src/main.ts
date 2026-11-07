import type {
  Project,
  Character,
  Binding,
  Animation,
  FrameRegion,
  CharacterState,
  RenderPlan,
  Resource,
} from "../../../packages/contracts/generated/index";
import { CharacterRenderer } from "../../../packages/renderer/src/index";
import { readRoute, writeRoute, type StudioRoute } from "./route";
import "./style.css";
type Meta = {
  id: string;
  packageId: string;
  legacyIds?: string[];
  revision: string;
  name: string;
  slug: string;
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
type StudioCharacter = Omit<Character, "resources" | "animations"> & {
  resources: Record<string, Resource>;
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
  contentHash: string;
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
  provider?: string;
  target?: string;
  revision: string;
  status: string;
  createdAt: string;
  development: boolean;
  logs: string;
  error?: string;
  downloads?: { name: string; bytes?: number }[];
  result?: { directory: string };
  github?: { repository: string; runUrl?: string };
  platforms?: Record<string, { status: string; conclusion?: string; imported?: boolean; importError?: string; steps?: {name: string; status: string; conclusion?: string}[] }>;
};
type GitHubState = { repository: string; connected: boolean; ready: boolean; private?: boolean; login?: string; defaultBranch?: string; workflowPresent?: boolean; credentialSource?: string; error?: string };
let github: GitHubState = { repository: "bright-k/desktop-buddy-studio", connected: false, ready: false }, githubError = "";
const app = document.querySelector<HTMLElement>("#app")!;
let projects: Meta[] = [],
  doc: Document | null = null,
  saved: Document | null = null,
  jobs: Job[] = [],
  tab = "overview",
  selectedClip = "idle",
  selectedBinding = 0,
  dirty = false,
  busy = false,
  archived = false;
let renderer: CharacterRenderer | undefined,
  previewTimer = 0,
  simulationTimers: number[] = [],
  cropImage: HTMLImageElement | undefined,
  crop: FrameRegion = { x: 0, y: 0, width: 256, height: 256 };
let config = { dataRoot: "", target: "macos-arm64", development: false },
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
  projectRequest = 0;
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
    ? `${Math.ceil(n / 1024)} KB`
    : `${(n / (1024 * 1024)).toFixed(1)} MB`;
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
  const response = await fetch(`/api/${path}`, {
    ...options,
    headers: {
      ...(options.body && typeof options.body === "string"
        ? { "Content-Type": "application/json" }
        : {}),
      ...options.headers,
    },
  });
  const value = await response.json();
  if (!response.ok) throw new Error(value.error ?? "요청에 실패했습니다.");
  for (const character of [value.character, value.draft?.character].filter(
    Boolean,
  )) {
    character.resources ??= {};
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
  if (message.includes("fps"))
    return "재생 속도(FPS)를 1~30 사이로 설정해주세요.";
  if (message.includes("ranges must not overlap"))
    return "입력 속도 구간이 겹칩니다. 각 이벤트의 최소·최대 속도를 조정해주세요.";
  if (message.includes("one-shot"))
    return "일회성 이벤트에 연결한 애니메이션의 반복 재생을 꺼주세요.";
  if (message.includes("sustain"))
    return "입력 속도·키 유지·드래그 동작은 반복 재생 애니메이션에 연결해주세요.";
  if (message.includes("INVALID_ICON"))
    return "아이콘은 512~1024px 정사각형 PNG여야 합니다.";
  if (message.includes("$.defaults.timing"))
    return "입력 중단·평가·안정 시간의 허용 범위를 확인해주세요. 평가 간격은 입력 중단 시간 이하여야 합니다.";
  if (message.includes("Another window") || message.includes("다른 창"))
    return "다른 창에서 수정되었습니다. 현재 편집 JSON을 내려받아 보관한 뒤 프로젝트를 다시 여세요.";
  return "문제가 있는 항목을 수정한 뒤 다시 저장하세요. 편집 내용은 그대로 유지됩니다.";
}
function validationPanel() {
  return validationError
    ? `<div id="validation-panel" class="validation-panel" role="alert" tabindex="-1"><strong>저장하기 전에 확인해주세요</strong><p>${esc(validationHint(validationError))}</p><details><summary>자세한 오류 보기</summary><pre>${esc(validationError)}</pre></details><button class="text-button" data-action="export-draft">현재 편집 JSON 내려받기</button></div>`
    : "";
}
function showValidationError(message: string) {
  validationError = message;
  const binding = message.match(/\$\.bindings\[(\d+)\]/),
    animation = message.match(/\$\.animations\.([a-zA-Z0-9_-]+)/);
  if (binding) {
    tab = "events";
    selectedBinding = Number(binding[1]);
  } else if (animation) {
    tab = "animations";
    selectedClip = animation[1];
  } else if (message.includes("$.defaults.timing")) tab = "timing";
  else if (message.includes("$.product")) tab = "overview";
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
const payload = () => ({
  revision: doc!.meta.revision,
  draftRevision,
  project: doc!.project,
  character: doc!.character,
  license: doc!.license,
});
function stopPreview() {
  renderer?.dispose();
  renderer = undefined;
  clearTimeout(previewTimer);
  simulationTimers.forEach(clearTimeout);
  simulationTimers = [];
}
function layout() {
  stopPreview();
  if (doc && !doc.character.animations[selectedClip]) selectedClip = "idle";
  app.innerHTML = `<aside class="sidebar"><a class="brand" href="/"><span class="brand-mark">b<span>•</span></span><span>buddy<span class="brand-studio">STUDIO</span></span></a><div class="workspace-label">LOCAL WORKSPACE <span class="local-dot"></span></div><div class="sidebar-heading"><h2>프로젝트 <span>${projects.filter((p) => p.archived === archived).length}</span></h2><button class="icon-button" data-action="new" aria-label="프로젝트 만들기">＋</button></div><div class="project-list">${
    projects
      .filter((p) => p.archived === archived)
      .map(
        (p, i) =>
          `<button class="project ${p.id === doc?.meta.id ? "selected" : ""}" data-action="open" data-id="${p.id}"><span class="project-avatar tone-${i % 4}">${esc(p.name[0])}</span><span><strong>${esc(p.name)}</strong><small title="${esc(p.id)}">${esc(p.id)}</small></span><span class="project-dot">${p.archived ? "◌" : "↗"}</span></button>`,
      )
      .join("") ||
    '<p class="empty-small">아직 프로젝트가 없습니다.<br>새로운 버디를 만들어보세요.</p>'
  }</div><button class="sidebar-new" data-action="new"><span>＋</span> 새 프로젝트</button><div class="sidebar-bottom"><button data-action="archive-list">${archived ? "← 작업 중인 프로젝트" : "▤ 보관한 프로젝트"}</button><span class="workspace-path" title="${esc(config.dataRoot)}">프로젝트는 이 컴퓨터에 저장됩니다.</span><span class="version">DESKTOP BUDDY STUDIO <span>v0.1</span></span></div></aside><div class="workspace">${doc ? editor() : welcome()}</div><div id="toast" class="toast" role="status"></div><dialog id="project-dialog"><form id="new-project-form"><span class="eyebrow">A NEW COMPANION</span><h2>새 버디 프로젝트</h2><p>이름과 고유한 패키지 ID를 정해주세요.</p><label class="field"><span>프로젝트 · 앱 이름</span><input name="name" required maxlength="80" value="My Buddy" autofocus></label><label class="field"><span>패키지 ID · 프로젝트 폴더와 URL</span><input name="packageId" required maxlength="128" value="io.desktopbuddy.app.mybuddy" placeholder="com.example.mybuddy" spellcheck="false" autocomplete="off"><small>예: com.example.typingcat · 생성 후 고정됩니다.</small></label><label class="field"><span>시작 템플릿</span><select name="template"><option value="typing-cat">Typing Cat · 키보드에 반응하는 고양이</option><option value="click-mascot">Click Mascot · 마우스에 반응하는 버디</option></select></label><p id="new-project-error" class="error-text" role="alert" hidden></p><footer><button type="button" data-action="close-dialog">취소</button><button class="primary" type="submit">프로젝트 만들기 ↗</button></footer></form></dialog>`;
  updateEditingStatus();
  if (doc) {
    if (!doc.character.animations[selectedClip]) selectedClip = "idle";
    const url = writeRoute(new URL(location.href), {
      projectId: doc.meta.id,
      tab: tab as StudioRoute["tab"],
      clip: selectedClip,
      bindingId: doc.project.bindings[selectedBinding]?.id ?? null,
    });
    if (url.href !== location.href) history.replaceState(null, "", url);
    startPreview();
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
        if (button.dataset.action !== "load-revisions") button.disabled = true;
      const saveButton = document.querySelector<HTMLButtonElement>(
        'button[data-action="save"]',
      );
      if (saveButton) saveButton.disabled = true;
    }
  }
}
function welcome() {
  return `<div class="welcome"><span class="eyebrow">YOUR DESKTOP, A LITTLE MORE ALIVE</span><h1>나만의 작은 동료를<br>만들어보세요.</h1><p>이미지와 GIF에 반응을 연결하면,<br>평범한 입력이 버디의 움직임이 됩니다.</p><button class="primary" data-action="new">＋ 첫 프로젝트 만들기</button><div class="welcome-cards"><article><span>01</span><h3>리소스 가져오기</h3><p>PNG, GIF, JPG, WebP, BMP</p></article><article><span>02</span><h3>반응 정의하기</h3><p>키보드 · 입력 속도 · 키워드 · 드래그</p></article><article><span>03</span><h3>앱으로 빌드하기</h3><p>프로젝트마다 독립된 앱과 패키지</p></article></div></div>`;
}
function editor() {
  return `<header class="topbar"><div class="breadcrumb">워크스페이스 <span>/</span> ${esc(doc!.project.product.name)}</div><div class="top-actions"><span id="save-state" role="status" aria-live="polite" class="save-state ${dirty ? "dirty" : ""}">${dirty ? "저장하지 않은 변경" : "모든 변경 저장됨"}</span><button id="cancel-edit" class="quiet" data-action="discard-draft" hidden>변경 취소</button><button data-action="validate">검증</button><button data-action="save" title="Ctrl / ⌘ + S">저장</button><button class="primary" data-action="tab" data-id="builds" ${doc!.meta.archived ? "disabled" : ""}>앱 빌드 <span>↗</span></button></div></header><div class="project-heading"><div><div class="eyebrow">BUDDY PROJECT <span class="pill">${doc!.meta.archived ? "보관됨" : "작업 중"}</span></div><h1>${esc(doc!.project.product.name)}</h1><p>리소스에 생동감을 더하고, 나만의 데스크톱 앱으로 만드세요.</p></div><div class="project-tools"><button class="quiet" data-action="clone">복제</button><button class="quiet" data-action="archive">${doc!.meta.archived ? "복원" : "보관"}</button></div></div><div class="editor-grid"><main class="editor"><nav class="tabs" aria-label="프로젝트 편집">${[
    ["overview", "프로젝트"],
    ["resources", "리소스"],
    ["animations", "애니메이션"],
    ["events", "이벤트"],
    ["timing", "타이밍"],
    ["builds", "빌드"],
  ]
    .map(
      ([id, label]) =>
        `<button data-action="tab" data-id="${id}" class="${tab === id ? "active" : ""}">${label}${id === "resources" ? ` <span>${Object.keys(doc!.character.resources ?? {}).length + 1}</span>` : ""}</button>`,
    )
    .join(
      "",
    )}</nav><div class="editor-content">${validationPanel()}${restoredDraft && dirty ? '<div class="draft-banner"><div><strong>임시 편집을 복원했습니다.</strong><p>저장하면 미리보기와 다음 빌드에 반영됩니다.</p></div><button class="small" data-action="discard-draft">편집 버리기</button></div>' : ""}${({ overview: overview, resources: resources, animations: animations, events: events, timing: timing, builds: builds }[tab] ?? overview)()}</div></main>${previewPanel()}</div><footer class="workspace-footer"><span><i class="local-dot"></i> 로컬 프로젝트 · ${esc(doc!.project.product.version)}</span><span title="${esc(doc!.contentHash)}">${esc(doc!.contentHash.slice(0, 12))} · ${date(doc!.meta.updatedAt)}</span></footer>`;
}
function title(kicker: string, name: string, description: string) {
  return `<div class="section-title"><span class="eyebrow">${kicker}</span><h2>${name}</h2><p>${description}</p></div>`;
}
function overview() {
  return `${title("PROJECT DETAILS", "버디의 기본 정보", "앱 이름, 표시 크기와 기본 동작을 설정합니다.")}<div class="card"><div class="form-grid">${field("앱 이름", "project.product.name")}${field("실행 파일 이름 (slug)", "project.product.slug", "text", jobs.some((j) => j.status === "succeeded") ? "readonly" : "")}${field("버전", "project.product.version")}${field("캐릭터 이름", "character.name")}</div><label class="field"><span>패키지 ID <small>프로젝트 폴더·URL·앱 식별자로 사용합니다.</small></span><input value="${esc(doc!.project.product.identifier)}" readonly></label></div><div class="card"><h3>데스크톱에서의 모습</h3><div class="form-grid">${field("기본 크기 배율", "project.defaults.scale", "number", 'min="0.25" max="3" step="0.05"')}${field("화면 여백 (DIP)", "project.defaults.position.marginDip", "number", 'min="0" max="256"')}${field("기준 프레임 너비 (px)", "character.sheet.frameWidth", "number", 'min="16" max="512"')}${field("기준 프레임 높이 (px)", "character.sheet.frameHeight", "number", 'min="16" max="512"')}</div><p class="hint">이미지와 GIF는 기준 프레임 안에 비율을 유지해 표시됩니다. 버디를 드래그하면 위치가 저장됩니다.</p>${check("항상 위에 표시", "project.defaults.alwaysOnTop")}${check("키보드 감지", "project.defaults.input.keyboard")}${check("마우스 감지", "project.defaults.input.mouse")}${check("키를 누르고 있을 때 이동 허용", "project.defaults.movementEnabled")}</div><div class="card"><h3>앱 아이콘 · 리소스 이용 조건</h3><div class="icon-upload"><img src="/api/projects/${doc!.meta.id}/source/assets/app-icon.png" alt="앱 아이콘"><div><strong>앱 아이콘</strong><p>512~1024px 정사각형 PNG</p><label class="button small">아이콘 교체<input type="file" id="icon-upload" accept="image/png" hidden></label></div></div><label class="field"><span>프로젝트 리소스 라이선스</span><textarea data-path="license" rows="5">${esc(doc!.license)}</textarea></label><p class="hint">가져온 리소스의 이용 조건과 출처를 여기에 모아두세요. 빌드 패키지에 함께 포함됩니다.</p></div>`;
}
function resources() {
  const entries = Object.entries(doc!.character.resources ?? {});
  return `${title("RESOURCE LIBRARY", "버디의 모든 리소스", "원본 파일을 프로젝트 안에 보관하고, 애니메이션에서 자유롭게 선택하세요.")}<label class="dropzone" id="dropzone"><input type="file" id="resource-upload" accept=".png,.jpg,.jpeg,.gif,.webp,.bmp" multiple hidden><span class="upload-symbol">↥</span><strong>이미지를 놓거나 클릭해서 가져오기</strong><span>PNG · JPG · GIF · WebP · BMP / 파일당 최대 16 MB</span></label><div class="resource-grid"><article class="resource-card"><div class="resource-art checker"><img src="${sourceUrl(doc!.character.image)}" alt="기본 스프라이트"></div><div><strong>기본 스프라이트</strong><span>PNG · ${doc!.character.sheet.width} × ${doc!.character.sheet.height}</span></div><small class="resource-id">sheet · 기본 제공</small></article>${entries
    .map(([id, r]) => {
      const f = r.image.slice(10),
        file = doc!.library[f];
      return `<article class="resource-card"><div class="resource-art checker"><img src="${resourceUrl(f)}" alt="${esc(id)}"></div><div><strong title="${esc(file?.originalName)}">${esc(file?.originalName ?? id)}</strong><span>${esc(f.split(".").at(-1)?.toUpperCase())} · ${file ? prettySize(file.bytes) : ""}</span></div><div class="resource-bottom"><small class="resource-id">${esc(id)}</small><button class="text-button danger" data-action="remove-resource" data-id="${esc(id)}">제외</button></div></article>`;
    })
    .join(
      "",
    )}</div><p class="hint">GIF는 프레임과 재생 시간을 읽어 이벤트마다 처음부터 재생합니다. 애니메이션 WebP/APNG는 GIF 또는 스프라이트로 변환해서 가져오세요.</p><div class="card"><h3>보관함 <span class="count">${Object.keys(doc!.library).length}</span></h3><p class="hint">구성에서 제외해도 원본은 남아 있습니다. 이전 버전 복원과 재사용에 사용할 수 있습니다.</p>${
    Object.values(doc!.library)
      .map((f) => {
        const used = Object.values(doc!.character.resources).some((r) => r.image === `resources/${f.name}`);
        return `<div class="library-row"><span>${esc(f.originalName)} <small>${prettySize(f.bytes)}</small></span><button class="text-button" data-action="reuse-resource" data-id="${f.name}" ${used ? "disabled" : ""}>${used ? "사용 중" : "구성에 추가"}</button></div>`;
      })
      .join("") || '<p class="hint">아직 가져온 원본 파일이 없습니다.</p>'
  }</div>`;
}
function animations() {
  const names = clips();
  if (!names.includes(selectedClip)) selectedClip = names[0];
  const a = doc!.character.animations[selectedClip],
    p = `character.animations.${selectedClip}`;
  const isGif =
    !!a.resource &&
    doc!.character.resources[a.resource].image.toLowerCase().endsWith(".gif");
  return `${title("ANIMATION CLIPS", "어떤 모습을 보여줄까요?", "한 장의 이미지, GIF 전체 또는 스프라이트의 특정 영역을 하나의 동작으로 정의합니다.")}<div class="subheading"><div class="clip-chips">${names.map((n) => `<button class="chip ${n === selectedClip ? "active" : ""}" data-action="clip" data-id="${esc(n)}">${esc(n)} <small>${doc!.character.animations[n].loop ? "↻" : "→"}</small></button>`).join("")}</div><button class="small" data-action="add-clip">＋ 동작</button></div><div class="card"><div class="card-heading"><h3>${esc(selectedClip)}</h3>${selectedClip !== "idle" ? '<button class="text-button danger" data-action="remove-clip">동작 삭제</button>' : '<span class="pill">기본 대기 동작</span>'}</div><label class="field"><span>표시할 리소스</span><select id="clip-resource">${option("sheet", "기본 PNG 스프라이트", a.resource ?? "sheet")}${Object.entries(
    doc!.character.resources ?? {},
  )
    .map(([id, r]) =>
      option(
        id,
        `${doc!.library[r.image.slice(10)]?.originalName ?? id} · ${id}`,
        a.resource ?? "sheet",
      ),
    )
    .join(
      "",
    )}</select></label><div class="form-grid">${field("재생 속도 (FPS)", `${p}.fps`, "number", 'min="1" max="30"')}${check("반복 재생", `${p}.loop`)}</div>${isGif ? check("GIF 원본 프레임 시간 사용", `${p}.useSourceTiming`) : a.resource ? "" : `<label class="field"><span>스프라이트 프레임 번호 (0부터, 쉼표로 구분)</span><input id="clip-frames" value="${a.frames.join(", ")}" placeholder="0, 1, 2, 3"></label>`}<p class="hint">일회성 이벤트는 반복을 끄고, 입력 속도·키 유지·드래그 동작은 반복을 켜세요. GIF는 프레임과 영역을 비워두면 전체를 사용합니다.</p></div>${isGif ? gifFramesEditor(a) : cropEditor(a)}`;
}
function gifFramesEditor(a: Animation) {
  return `<div class="card"><h3>GIF 프레임 선택</h3><p class="hint">비워두면 전체 프레임을 원본 순서로 사용합니다. 특정 프레임만 쓰려면 0부터 시작하는 번호를 쉼표로 구분하세요. GIF는 합성된 원본 프레임을 사용하므로 영역 자르기를 적용하지 않습니다.</p><label class="field"><span>프레임 번호 (선택)</span><input id="clip-frames" value="${a.frames.join(", ")}" placeholder="전체 프레임"></label></div>`;
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
  keySequence: "숨겨진 키워드",
  mouseDown: "마우스 누름",
  mouseUp: "마우스 뗌",
  wheel: "휠",
  cursorMove: "커서 따라보기",
  drag: "버디 드래그",
};
function events() {
  const bindings = doc!.project.bindings;
  if (selectedBinding >= bindings.length)
    selectedBinding = Math.max(0, bindings.length - 1);
  const b = bindings[selectedBinding];
  return `${title("EVENT MAPPINGS", "입력에 반응을 연결하세요.", "다른 앱에서 작업하는 동안에도 입력을 감지합니다. 버디를 드래그하는 동안의 동작도 지정할 수 있습니다.")}<div class="subheading"><span class="hint">${bindings.length}개의 이벤트 · 위쪽이 같은 우선순위에서 먼저 적용됩니다.</span><button class="small" data-action="add-binding">＋ 이벤트</button></div><div class="binding-list">${bindings.map((b, i) => `<button class="binding-row ${i === selectedBinding ? "active" : ""}" data-action="binding" data-id="${i}"><span class="binding-icon">${b.trigger.type === "typingRate" ? "↯" : b.trigger.type === "drag" ? "✥" : "⌘"}</span><span><strong>${esc(b.id)}</strong><small>${triggerNames[b.trigger.type]} ${esc((b.trigger as any).code ?? (b.trigger as any).keyword ?? "")}</small></span><span class="binding-arrow">→</span><span class="binding-animation">${esc((b.effect as any).animation ?? "방향 전환")}</span></button>`).join("")}</div>${b ? bindingEditor(b) : '<div class="card empty-small">이벤트를 추가해서 동작과 연결하세요.</div>'}`;
}
function bindingEditor(b: Binding) {
  const t: any = b.trigger,
    e: any = b.effect,
    p = `project.bindings.${selectedBinding}`;
  return `<div class="card"><div class="card-heading"><h3>이벤트 편집</h3><button class="text-button danger" data-action="remove-binding">삭제</button></div><div class="form-grid">${field("이벤트 ID", `${p}.id`)}<label class="field"><span>발생 조건</span><select id="trigger-type">${Object.entries(
    triggerNames,
  )
    .map(([k, v]) => option(k, v, t.type))
    .join(
      "",
    )}</select></label></div>${["keyDown", "keyUp", "whileKeyDown"].includes(t.type) ? `${field("물리 키 코드", `${p}.trigger.code`)}<p class="hint">예: Space, Enter, KeyA, ArrowRight. 키 누름·뗌에 *를 쓰면 일반 키 전체에 반응합니다.</p>${t.type === "keyDown" ? check("키를 길게 눌러 반복되는 입력에도 반응", `${p}.trigger.repeat`) : ""}` : ""}${t.type === "typingRate" ? `<div class="form-grid">${field("최소 입력 속도 (키/초)", `${p}.trigger.minKps`, "number", 'min="0.5" step="0.5"')}<label class="field"><span>최대 입력 속도 (비우면 제한 없음)</span><input id="max-kps" type="number" min="0.5" step="0.5" value="${t.maxKps ?? ""}"></label></div><p class="hint">최소값 이상, 최대값 미만에 적용합니다. 속도 구간은 서로 겹칠 수 없습니다.</p>` : ""}${t.type === "keySequence" ? `${field("키워드", `${p}.trigger.keyword`)}<div class="form-grid"><label class="field"><span>키 배열</span><select data-path="${p}.trigger.layout">${option("ko-KR-2set", "한글 두벌식", t.layout)}${option("en-US-qwerty", "영문 QWERTY", t.layout)}</select></label>${field("입력 제한 시간 (ms)", `${p}.trigger.timeoutMs`, "number", 'min="500" max="10000"')}</div><p class="hint">완성 문자열을 읽지 않고, 지정한 키워드의 물리 키 순서를 검사합니다.</p>` : ""}${["mouseDown", "mouseUp"].includes(t.type) ? `<label class="field"><span>마우스 버튼</span><select data-path="${p}.trigger.button">${["primary", "secondary", "middle"].map((v, i) => option(v, ["왼쪽", "오른쪽", "가운데"][i], t.button)).join("")}</select></label>` : ""}${t.type === "wheel" ? `<label class="field"><span>방향</span><select data-path="${p}.trigger.direction">${["up", "down", "left", "right"].map((v) => option(v, v, t.direction)).join("")}</select></label>` : ""}${t.type === "whileKeyDown" ? `<label class="field"><span>동작 종류</span><select id="held-effect">${option("sustainAnimation", "키를 누르는 동안 애니메이션", e.type)}${option("move", "키를 누르는 동안 이동", e.type)}</select></label>` : ""}${
    e.type !== "faceCursor"
      ? `<label class="field"><span>표시할 애니메이션</span><select data-path="${p}.effect.animation">${e.type === "move" ? option("", "기본 대기 모습", e.animation ?? "") : ""}${clips()
          .filter((n) =>
            e.type === "playAnimation"
              ? !doc!.character.animations[n].loop
              : doc!.character.animations[n].loop,
          )
          .map((n) => option(n, n, e.animation ?? ""))
          .join("")}</select></label>`
      : '<p class="hint">커서가 있는 방향으로 캐릭터를 좌우 반전합니다.</p>'
  }${e.type === "playAnimation" ? `<div class="form-grid">${field("우선순위 (0~100)", `${p}.effect.priority`, "number", 'min="0" max="100"')}${field("재반응 대기 (ms)", `${p}.effect.cooldownMs`, "number", 'min="0" max="10000"')}<label class="field"><span>표시 시간 (ms, 비우면 동작 길이)</span><input id="duration-ms" type="number" min="1" max="60000" value="${e.durationMs ?? ""}"></label><label class="field"><span>재입력 시</span><select data-path="${p}.effect.retrigger">${option("restart", "처음부터 재생", e.retrigger)}${option("ignore", "현재 재생 유지", e.retrigger)}</select></label></div>${["keyDown", "mouseDown"].includes(t.type) ? check("키·버튼을 떼면 즉시 대기로 복귀", `${p}.effect.stopOnRelease`) : ""}` : ""}${e.type === "move" ? `<div class="form-grid">${field("X 방향 (-1~1)", `${p}.effect.x`, "number", 'min="-1" max="1" step="0.1"')}${field("Y 방향 (-1~1)", `${p}.effect.y`, "number", 'min="-1" max="1" step="0.1"')}${field("이동 속도 (DIP/초)", `${p}.effect.speedDipPerSecond`, "number", 'min="1" max="1000"')}</div>` : ""}</div>`;
}
function timing() {
  return `${title("RESPONSE TIMING", "버디의 반응 리듬", "입력을 멈춘 뒤 계속 움직이는 시간을 줄이고, 속도에 따른 전환을 조절하세요.")}<div class="timing-highlight"><span class="timing-icon">◷</span><div><h3>입력 중단 → 대기 전환</h3><p>마지막 일반 키 입력 후 ${doc!.project.defaults.timing.idleTimeoutMs}ms에 속도 애니메이션을 끝냅니다.</p></div><strong>${doc!.project.defaults.timing.idleTimeoutMs}<small>ms</small></strong></div><div class="card">${field("입력 중단 판정 (ms)", "project.defaults.timing.idleTimeoutMs", "number", 'min="50" max="5000" step="10"')}<p class="hint">기본 250ms. 더 빠른 복귀는 150~200ms, 여유 있는 복귀는 400~600ms를 사용해보세요. 일회성 동작의 길이는 이벤트의 표시 시간에서 조절합니다.</p><div class="form-grid">${field("속도 측정 구간 (ms)", "project.defaults.timing.rateWindowMs", "number", 'min="100" max="10000" step="50"')}${field("속도 평가 간격 (ms)", "project.defaults.timing.rateEvaluationMs", "number", 'min="10" max="250" step="10"')}${field("속도 구간 안정 시간 (ms)", "project.defaults.timing.rateStabilityMs", "number", 'min="0" max="2000" step="10"')}</div><p class="hint">속도 = 측정 구간의 키 입력 수 ÷ 구간(초). 안정 시간이 지나야 새 속도 구간으로 바뀝니다. 중단 판정은 안정 시간과 관계없이 즉시 대기로 돌아갑니다.</p></div><div class="card"><h3>시작 프리셋</h3><div class="preset-row"><button data-action="timing-preset" data-id="snappy">빠릿하게 <small>중단 150ms · 안정 50ms</small></button><button data-action="timing-preset" data-id="balanced">기본 <small>중단 250ms · 안정 100ms</small></button><button data-action="timing-preset" data-id="relaxed">부드럽게 <small>중단 500ms · 안정 200ms</small></button></div></div>`;
}
function builds() {
  const platform = config.target === "macos-arm64" ? "macOS" : "Windows";
  return `${title("BUILD & HISTORY", "프로젝트를 앱으로", "같은 프로젝트 정의와 리소스로 macOS·Windows 앱을 만듭니다.")}<div class="platform-grid">${[
    ["macos-arm64", "macOS · Apple Silicon", ".app · DMG · ZIP"],
    ["windows-x64", "Windows · x64", "설치 파일 .exe"],
  ].map(([target, label, format]) => {
    const enabled = doc!.project.build.targets.includes(target as any);
    const local = enabled && config.target === target;
    return `<div class="platform-card ${local ? "current" : ""}"><span class="pill">${!enabled ? "구성에서 제외됨" : github.ready ? "Actions에서 빌드" : local ? "이 컴퓨터에서 빌드 가능" : "GitHub 연결 필요"}</span><h3>${label}</h3><p>${format}</p></div>`;
  }).join("")}</div><section id="github-panel">${githubPanel()}</section><div class="build-banner"><div><h3>macOS + Windows 함께 빌드</h3><p>현재 저장 버전과 참조 리소스·고정 엔진을 ${esc(github.repository)}에 업로드합니다. ${github.connected ? github.private ? "비공개 저장소입니다." : "공개 저장소이므로 업로드한 정의·이미지는 공개됩니다." : "먼저 저장소를 연결하세요."} 두 OS의 진행 상황과 검증한 결과물을 아래 이력에 가져옵니다.</p></div><button class="primary" data-action="build-remote" ${github.connected && github.ready && !doc!.meta.archived ? "" : "disabled"}>업로드하고 두 플랫폼 빌드 ↗</button></div><p class="hint">Actions 개발용 패키지 · 공개 배포 서명·공증과 실제 입력·설치 검증은 별도입니다. 비공개 저장소의 Actions 사용은 GitHub 요금제 한도를 따릅니다.</p><details class="card cross-platform"><summary>이 컴퓨터에서만 빌드 · 프로젝트 내보내기</summary><p>${platform}용 패키지를 로컬에서 만듭니다. 프로젝트를 GitHub에 업로드하지 않습니다.</p><div class="row-actions"><button class="small" data-action="build">${platform} 앱 로컬 빌드 ↗</button><button class="small" data-action="export-project">저장한 프로젝트 내보내기 ↓</button></div></details><div id="build-history">${buildHistory()}</div><details class="card"><summary>저장한 구성 버전 · 복원</summary><p class="hint">모든 저장은 새로운 버전입니다. 이전 리소스와 정의를 함께 복원할 수 있습니다.</p><button class="small" data-action="load-revisions">저장 이력 보기</button><div id="revision-history"></div></details>`;
}
function githubPanel() {
  return `<div class="card github-connection"><div class="card-heading"><h3>GitHub Actions</h3><span class="pill">${github.connected ? github.ready ? "빌드 준비 완료" : "워크플로 준비 필요" : "연결 안 됨"}</span></div>${github.connected ? `<p><strong>${esc(github.repository)}</strong> · ${github.private ? "비공개" : "공개"} · ${esc(github.login)}</p>${github.ready ? "" : `<p>기본 브랜치 ${esc(github.defaultBranch)}에 Studio 전용 워크플로 파일 하나를 ${github.workflowPresent ? "업데이트" : "추가"}합니다. 프로젝트는 빌드할 때 별도 브랜치에 업로드합니다.</p><button class="small" data-action="github-setup">워크플로 ${github.workflowPresent ? "업데이트" : "준비"}</button>`}` : `<p>Git 인증 또는 환경 변수의 GitHub 토큰으로 연결합니다. 다른 컴퓨터에서 빌드를 수행하므로 이 Mac에서도 Windows 결과물을 만들 수 있습니다.</p>`}<p id="github-error" class="error-text" role="alert" ${githubError || github.error ? "" : "hidden"}>${esc(githubError || github.error)}</p><details ${github.connected ? "" : "open"}><summary>${github.connected ? "연결 변경" : "저장소와 인증"}</summary><label class="field"><span>GitHub 저장소</span><input id="github-repository" value="${esc(github.repository)}" placeholder="owner/repository" spellcheck="false" autocomplete="off"></label><label class="field"><span>토큰 (선택 · 기존 Git 인증을 쓰면 비워두세요)</span><input id="github-token" type="password" autocomplete="off" placeholder="이 Studio 실행 중에만 보관"></label><p class="hint">선택한 저장소의 Contents·Actions·Workflows 읽기/쓰기 권한이 필요합니다. 직접 입력한 토큰은 디스크·프로젝트·URL에 저장하지 않습니다. 연결 해제는 원격 빌드를 취소하지 않습니다.</p><div class="row-actions"><button class="small" data-action="github-connect">저장소 연결</button>${github.connected ? '<button class="small quiet" data-action="github-disconnect">연결 해제</button>' : ""}</div></details></div>`;
}
const remoteActive = ["preparing", "queued", "running", "downloading", "awaiting_auth", "dispatch_unknown", "cancelling"];
function remoteJob(j: Job) {
  const states: Record<string, string> = { queued: "대기 중", waiting: "대기 중", pending: "대기 중", in_progress: "빌드 중", completed: "완료", success: "빌드 완료", failure: "실패", cancelled: "취소됨", skipped: "건너뜀", timed_out: "시간 초과", action_required: "GitHub 확인 필요" };
  return `<div class="remote-platforms">${Object.entries(j.platforms ?? {}).map(([target, p]) => `<div><strong>${target === "macos-arm64" ? "macOS · Apple Silicon" : "Windows · x64"}</strong><span class="pill">${p.imported ? "✓ 가져오기 완료" : esc(states[p.conclusion || p.status] || p.status)}</span>${p.importError ? `<p class="error-text">${esc(p.importError)}</p>` : ""}</div>`).join("")}</div><div class="row-actions">${j.github?.runUrl ? `<a class="button small" target="_blank" rel="noreferrer" href="${esc(j.github.runUrl)}">Actions에서 상세 로그 ↗</a>` : ""}${["partial", "import_failed"].includes(j.status) ? `<button class="small" data-action="sync-build" data-id="${j.id}">결과 다시 가져오기</button>` : ""}</div>`;
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
    dispatch_unknown: "Actions 실행 확인 중", partial: "일부 결과 완료", import_failed: "가져오기 재시도 필요",
  };
  return (
    jobs
      .map(
        (j) =>
          `<article class="build-card" data-job-id="${j.id}"><div class="card-heading"><div><span class="job-status ${j.status}">${names[j.status] ?? esc(j.status)}</span><strong>${date(j.createdAt)}</strong></div>${(j.provider === "github" ? remoteActive : ["queued", "running"]).includes(j.status) ? `<button class="small" data-action="cancel-build" data-id="${j.id}">빌드 취소</button>` : `<span class="hint">${j.development ? "개발용" : "배포용"}</span>`}</div><p class="build-revision">${j.provider === "github" ? "Actions / macOS + Windows" : j.target === "windows-x64" ? "Windows x64" : j.target === "macos-arm64" ? "macOS arm64" : "플랫폼 미기록"} · 구성 ${j.revision.slice(0, 8)} · ${j.id.slice(0, 8)}</p>${j.provider === "github" ? remoteJob(j) : ""}${j.result ? `<p class="artifact-path">${esc(j.result.directory)}</p>` : ""}${j.error ? `<p class="error-text">${esc(j.error)}</p>` : ""}${j.downloads?.length ? `<div class="downloads">${j.downloads.map((d) => `<a class="button small ${/\.zip$|\.dmg$|\.exe$|\.tar\.gz$/.test(d.name) ? "download-package" : ""}" href="/api/projects/${doc!.meta.id}/download/${j.id}/${encodeURIComponent(d.name)}" download>${esc(d.name)} ↓</a>`).join("")}</div>` : ""}${j.logs ? `<details ${j.status === "running" ? "open" : ""}><summary>빌드 로그</summary><pre>${esc(j.logs)}</pre></details>` : ""}</article>`,
      )
      .join("") ||
    '<div class="empty-build"><span>▤</span><h3>아직 빌드한 앱이 없습니다.</h3><p>구성을 저장하고 첫 번째 패키지를 만들어보세요.</p></div>'
  );
}
function previewPanel() {
  return `<aside class="preview-panel"><div class="preview-heading"><h2>미리보기</h2><span class="preview-live"><i></i> PREVIEW</span></div><div class="preview-stage checker"><div class="preview-caption">YOUR LITTLE COMPANION</div><canvas id="preview-canvas" aria-label="버디 애니메이션 미리보기"></canvas><div class="preview-shadow"></div><span id="preview-state">idle</span></div><div class="preview-controls"><p id="preview-notice" class="hint"></p><button id="preview-save" class="primary small full-width" data-action="save" hidden>저장하고 미리보기</button><label class="field"><span>저장된 구성 · 애니메이션 재생</span><select id="preview-clip">${clips()
    .map((n) => option(n, n, selectedClip))
    .join(
      "",
    )}</select></label><button class="small full-width" data-action="replay">↻ 처음부터 재생</button><div class="preview-divider"></div><h3>이벤트 테스트</h3><p class="hint">저장된 구성으로 실제 동작 엔진을 실행합니다.</p><label class="field"><span>테스트할 이벤트</span><select id="test-binding">${doc!.project.bindings.map((b, i) => option(String(i), `${triggerNames[b.trigger.type]} · ${b.id}`, String(selectedBinding))).join("")}</select></label><button class="small full-width" data-action="test-event" ${doc!.project.bindings.length ? "" : "disabled"}>이벤트 실행 →</button><div class="speed-test"><span>타이핑 속도</span><div><button data-action="test-speed" data-id="2">2 키/초</button><button data-action="test-speed" data-id="8">8 키/초</button></div><small>2초 입력 후 멈춤 → 대기 복귀 확인</small></div><p class="preview-note">웹 미리보기는 선택한 테스트 입력만 사용합니다. 완성된 앱은 다른 앱에서 작업할 때도 반응합니다.</p><div class="preview-divider"></div><div class="project-facts"><span>리소스<strong>${Object.keys(doc!.character.resources ?? {}).length + 1}</strong></span><span>동작<strong>${clips().length}</strong></span><span>이벤트<strong>${doc!.project.bindings.length}</strong></span></div></div></aside>`;
}
function startPreview() {
  if (!doc || !saved) return;
  const canvas = document.querySelector<HTMLCanvasElement>("#preview-canvas");
  if (!canvas) return;
  const imageUrls = Object.fromEntries(
    saved.images.map((name) => [
      name,
      `/api/projects/${saved!.meta.id}/frames/${saved!.meta.revision}/${name}`,
    ]),
  );
  renderer = new CharacterRenderer(
    canvas,
    saved.character,
    imageUrls,
    () => {},
    saved.renderPlan,
  );
  renderer.resize(240, 240);
  playClip(selectedClip);
  if (tab === "animations") setupCrop();
}
function playClip(name: string) {
  if (!renderer || !saved) return;
  simulationTimers.forEach(clearTimeout);
  simulationTimers = [];
  const state: CharacterState = {
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
  renderer.update(state, 1);
  const label = document.querySelector("#preview-state");
  if (label)
    label.textContent = saved.character.animations[name]
      ? name
      : "저장 후 미리보기";
}
async function save(icon?: string) {
  if (!doc || busy || !dirty) return;
  busy = true;
  app.inert = true;
  try {
    await flushDraft();
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
    projects = await api("projects");
    jobs = await projectApi("builds");
    layout();
    toast("구성과 리소스를 새 버전으로 저장했습니다.");
  } catch (error) {
    showValidationError((error as Error).message);
    throw error;
  } finally {
    busy = false;
    app.inert = false;
  }
}
async function openProject(id: string, route?: StudioRoute) {
  if (dirty) await flushDraft();
  const request = ++projectRequest;
  const workspace = document.querySelector<HTMLElement>(".workspace");
  if (workspace) {
    workspace.inert = true;
    workspace.setAttribute("aria-busy", "true");
  }
  try {
    const [value, nextJobs] = await Promise.all([
      api<Document>(`projects/${id}`),
      api<Job[]>(`projects/${id}/builds`),
    ]);
    if (request !== projectRequest) return;
    saved = structuredClone(value);
    doc = value;
    jobs = nextJobs;
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
    layout();
  } finally {
    if (request === projectRequest) {
      const workspace = document.querySelector<HTMLElement>(".workspace");
      if (workspace) {
        workspace.inert = false;
        workspace.removeAttribute("aria-busy");
      }
    }
  }
}
async function ensureSaved() {
  if (dirty) await save();
}
async function upload(files: FileList | File[], icon = false) {
  if (!doc || busy) return;
  if (doc.meta.archived)
    throw new Error("보관된 프로젝트는 복원 후 수정하세요.");
  busy = true;
  app.inert = true;
  const errors: string[] = [],
    list = Array.from(files);
  let imported = 0,
    selectedIcon: string | undefined;
  try {
    for (const [index, file] of list.entries()) {
      toast(`리소스를 가져오는 중 (${index + 1}/${list.length})`);
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
    busy = false;
    app.inert = false;
    layout();
  }
  if (selectedIcon) await save(selectedIcon);
  if (errors.length)
    toast(
      `${imported}개 가져옴 · ${errors.length}개 실패\n${errors.join("\n")}`,
      true,
    );
  else if (!icon)
    toast(`${imported}개 리소스를 가져왔습니다. 동작에 연결한 뒤 저장하세요.`);
}
function addResource(filename: string) {
  doc!.character.resources ??= {};
  if (
    Object.values(doc!.character.resources).some(
      (r) => r.image === `resources/${filename}`,
    )
  )
    return;
  let n = 1;
  while (doc!.character.resources[`resource-${n}`]) n++;
  doc!.character.resources[`resource-${n}`] = {
    image: `resources/${filename}`,
  };
  markDirty();
}
function chooseDefaultAnimation(loop: boolean) {
  return (
    clips().find((n) => doc!.character.animations[n].loop === loop) ?? "idle"
  );
}
function changeTrigger(type: string) {
  const b = doc!.project.bindings[selectedBinding],
    t: any = { type };
  if (["keyDown", "keyUp", "whileKeyDown"].includes(type)) t.code = "Space";
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
  if (["mouseDown", "mouseUp"].includes(type)) t.button = "primary";
  if (type === "wheel") t.direction = "up";
  b.trigger = t;
  b.effect =
    type === "cursorMove"
      ? { type: "faceCursor" }
      : ["typingRate", "whileKeyDown", "drag"].includes(type)
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
    : sourceUrl(doc!.character.image);
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
) {
  const projectId = doc!.meta.id;
  await ensureSaved();
  const timeline = await projectApi<{ atMs: number; state: CharacterState }[]>(
    "simulate",
    post({ events, untilMs }),
  );
  if (doc?.meta.id !== projectId) return;
  simulationTimers.forEach(clearTimeout);
  simulationTimers = [];
  const epoch = currentEpoch++;
  let previous = "";
  for (const frame of timeline) {
    const key = `${frame.state.actionInstanceId}:${frame.state.animation}`;
    if (key === previous) continue;
    previous = key;
    simulationTimers.push(
      window.setTimeout(() => {
        renderer?.update({ ...frame.state, epoch }, frame.atMs);
        document.querySelector("#preview-state")!.textContent =
          `${frame.state.animation} · ${frame.atMs}ms`;
      }, frame.atMs),
    );
  }
  toast("실제 동작 엔진으로 이벤트를 재생합니다.");
}
async function testBinding(index: number) {
  const projectId = doc!.meta.id;
  const b = doc!.project.bindings[index];
  if (!b) return;
  if (b.effect.type === "move" && !doc!.project.defaults.movementEnabled)
    throw new Error("프로젝트 탭에서 키 유지 이동을 먼저 켜주세요.");
  if (
    ["keyDown", "keyUp", "whileKeyDown", "typingRate", "keySequence"].includes(
      b.trigger.type,
    ) &&
    !doc!.project.defaults.input.keyboard
  )
    throw new Error("프로젝트 탭에서 키보드 감지를 먼저 켜주세요.");
  if (
    ["mouseDown", "mouseUp", "wheel", "cursorMove"].includes(b.trigger.type) &&
    !doc!.project.defaults.input.mouse
  )
    throw new Error("프로젝트 탭에서 마우스 감지를 먼저 켜주세요.");
  const t: any = b.trigger,
    events: { atMs: number; event: any }[] = [];
  let end = 2000;
  const key = (code: string, atMs: number) => {
    events.push(
      { atMs, event: { type: "keyDown", code, repeat: false } },
      { atMs: atMs + 35, event: { type: "keyUp", code } },
    );
  };
  switch (t.type) {
    case "keyDown":
    case "keyUp":
    case "whileKeyDown":
      events.push(
        {
          atMs: 100,
          event: {
            type: "keyDown",
            code: t.code === "*" ? "KeyA" : t.code,
            repeat: false,
          },
        },
        {
          atMs: 800,
          event: { type: "keyUp", code: t.code === "*" ? "KeyA" : t.code },
        },
      );
      break;
    case "mouseDown":
    case "mouseUp":
      events.push(
        { atMs: 100, event: { type: "mouseDown", button: t.button } },
        { atMs: 800, event: { type: "mouseUp", button: t.button } },
      );
      break;
    case "wheel":
      events.push({ atMs: 100, event: t });
      break;
    case "cursorMove":
      events.push({ atMs: 100, event: { type: "cursorMove", x: 0, y: 0 } });
      break;
    case "drag":
      events.push(
        { atMs: 100, event: { type: "dragStart" } },
        { atMs: 1200, event: { type: "dragEnd" } },
      );
      break;
    case "typingRate":
      return testSpeed(t.minKps + 0.5);
    case "keySequence": {
      const steps = await compileSequence(index);
      if (doc?.meta.id !== projectId) return;
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
      end = at + 2500;
      break;
    }
  }
  await simulate(events, end);
}
// Keyword compilation is supplied by the Rust validator, never duplicated here.
async function compileSequence(
  index: number,
): Promise<{ code: string; shift: boolean }[]> {
  await ensureSaved();
  return projectApi("sequence", post({ index }));
}
async function testSpeed(rate: number) {
  if (!doc!.project.defaults.input.keyboard)
    throw new Error("프로젝트 탭에서 키보드 감지를 먼저 켜주세요.");
  const events = [];
  for (let at = 0; at < 2000; at += 1000 / rate)
    events.push(
      {
        atMs: Math.round(at),
        event: { type: "keyDown", code: "KeyA", repeat: false },
      },
      { atMs: Math.round(at) + 20, event: { type: "keyUp", code: "KeyA" } },
    );
  events.sort((a, b) => a.atMs - b.atMs);
  await simulate(events, 3500);
}
async function action(name: string, id?: string) {
  if (busy) return;
  if (
    ["clone", "archive", "restore", "discard-draft", "cancel-build"].includes(
      name,
    )
  ) {
    if (["clone", "archive"].includes(name) && dirty) await save();
    busy = true;
    app.inert = true;
    try {
      return await performAction(name, id);
    } finally {
      busy = false;
      app.inert = false;
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
      busy = true; app.inert = true; githubError = "";
      try {
        github = await api<GitHubState>(`github/${name.slice(7)}`, post({ repository, ...(token ? { token } : {}) }));
        layout(); toast(name === "github-setup" ? "두 플랫폼 빌드 워크플로를 준비했습니다." : name === "github-connect" ? "GitHub 저장소를 연결했습니다." : "GitHub 연결을 해제했습니다.");
      } catch (e) {
        githubError = (e as Error).message;
        const error = document.querySelector<HTMLElement>("#github-error");
        if (error) { error.textContent = githubError; error.hidden = false; }
        throw e;
      } finally { busy = false; app.inert = false; }
      break;
    }
    case "sync-build":
      await projectApi(`builds/${id}/sync`, post({}));
      jobs = await projectApi("builds"); layout();
      toast("Actions의 결과물을 다시 확인합니다. 새 빌드는 시작하지 않습니다.");
      break;
    case "build-remote": {
      await ensureSaved();
      const projectId = doc!.meta.id;
      const job = await projectApi<Job>("builds", post({ revision: doc!.meta.revision, provider: "github", acknowledgeUpload: true }));
      const next = await api<Job[]>(`projects/${projectId}/builds`);
      if (doc?.meta.id !== projectId) return;
      jobs = next; tab = "builds"; layout();
      toast(`두 플랫폼 빌드 ${job.id.slice(0,8)}를 준비합니다. 편집을 계속해도 빌드 구성은 고정됩니다.`);
      break;
    }
    case "export-project": {
      await ensureSaved();
      busy = true;
      app.inert = true;
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
        busy = false;
        app.inert = false;
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
      if (!confirm("임시 편집을 버리고 마지막 저장 구성으로 돌아갈까요?"))
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
      layout();
      break;
    case "save":
      await save();
      break;
    case "validate":
      busy = true;
      app.inert = true;
      try {
        await projectApi("validate", post(payload()));
        validationError = "";
        document.querySelector("#validation-panel")?.remove();
      } catch (error) {
        showValidationError((error as Error).message);
        throw error;
      } finally {
        busy = false;
        app.inert = false;
      }
      toast("구성, 리소스와 이벤트 연결을 검증했습니다.");
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
      selectedClip = id!;
      layout();
      break;
    case "binding":
      selectedBinding = Number(id);
      layout();
      break;
    case "replay":
      playClip(
        (document.querySelector("#preview-clip") as HTMLSelectElement).value,
      );
      break;
    case "add-clip": {
      const n = prompt("새 동작 이름 (영문, 숫자, -, _)", "new-animation");
      if (!n) return;
      if (!/^[a-zA-Z0-9_-]{1,64}$/.test(n) || doc!.character.animations[n])
        throw new Error("겹치지 않는 영문 동작 이름을 입력하세요.");
      doc!.character.animations[n] = {
        frames: [0],
        fps: 12,
        loop: false,
        resource: null,
        regions: [],
        useSourceTiming: true,
      };
      selectedClip = n;
      markDirty();
      layout();
      break;
    }
    case "remove-clip":
      if (
        doc!.project.bindings.some(
          (b) => (b.effect as any).animation === selectedClip,
        )
      )
        throw new Error("이 동작을 사용하는 이벤트를 먼저 수정하세요.");
      delete doc!.character.animations[selectedClip];
      selectedClip = "idle";
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
    case "test-event":
      await testBinding(
        Number(
          (document.querySelector("#test-binding") as HTMLSelectElement).value,
        ),
      );
      break;
    case "test-speed":
      await testSpeed(Number(id));
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
    case "load-revisions": {
      const history = await projectApi<
        {
          revision: string;
          name: string;
          version: string;
          updatedAt: string;
        }[]
      >("revisions");
      document.querySelector("#revision-history")!.innerHTML = history
        .map(
          (r) =>
            `<div class="library-row"><span>${date(r.updatedAt)} · ${esc(r.name)} v${esc(r.version)}</span><button class="text-button" data-action="restore" data-id="${r.revision}" ${r.revision === doc!.meta.revision ? "disabled" : ""}>${r.revision === doc!.meta.revision ? "현재 버전" : "복원"}</button></div>`,
        )
        .join("");
      break;
    }
    case "restore":
      if (
        dirty &&
        !confirm("저장하지 않은 변경을 버리고 이전 구성을 복원할까요?")
      )
        return;
      doc = await projectApi(
        "restore",
        post({ revision: id, expectedRevision: doc!.meta.revision }),
      );
      saved = structuredClone(doc);
      dirty = false;
      draftRevision = null;
      draftContent = "";
      draftState = "idle";
      restoredDraft = false;
      validationError = "";
      projects = await api("projects");
      layout();
      toast("선택한 구성과 리소스를 복원했습니다.");
      break;
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
app.addEventListener("input", (event) => {
  const input = event.target as HTMLInputElement;
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
  if (busy) return;
  const element = event.target as HTMLFormElement,
    form = new FormData(element),
    error = element.querySelector<HTMLElement>("#new-project-error")!;
  void (async () => {
    busy = true;
    app.inert = true;
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
      tab = "overview";
      await openProject(value.meta.id);
      toast("새 프로젝트를 만들었습니다.");
    } catch (cause) {
      error.textContent = (cause as Error).message;
      error.hidden = false;
    } finally {
      busy = false;
      app.inert = false;
    }
  })();
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
  try {
    const nextConnection = await api<GitHubState>("github");
    if (JSON.stringify(nextConnection) !== JSON.stringify(github)) {
      github = nextConnection;
      const panel = document.querySelector("#github-panel");
      if (panel) panel.innerHTML = githubPanel();
      for (const button of document.querySelectorAll<HTMLButtonElement>('[data-action="build-remote"]'))
        button.disabled = !(github.connected && github.ready) || !!doc?.meta.archived;
    }
    if (doc) {
      const id = doc.meta.id,
        next = await projectApi<Job[]>("builds");
      if (doc?.meta.id === id) {
        const changed = JSON.stringify(jobs) !== JSON.stringify(next);
        jobs = next;
        const el = document.querySelector("#build-history");
        if (el && changed) {
          const states = new Map(
            [...el.querySelectorAll<HTMLElement>("[data-job-id]")].map(
              (card) => {
                const details = card.querySelector("details"),
                  pre = card.querySelector("pre");
                return [
                  card.dataset.jobId,
                  {
                    open: details?.open,
                    top: pre?.scrollTop ?? 0,
                    bottom: pre
                      ? pre.scrollHeight - pre.clientHeight - pre.scrollTop < 3
                      : false,
                  },
                ] as const;
              },
            ),
          );
          el.innerHTML = buildHistory();
          for (const card of el.querySelectorAll<HTMLElement>(
            "[data-job-id]",
          )) {
            const state = states.get(card.dataset.jobId),
              details = card.querySelector("details"),
              pre = card.querySelector("pre");
            if (state && details && state.open !== undefined)
              details.open = state.open;
            if (state && pre)
              pre.scrollTop = state.bottom ? pre.scrollHeight : state.top;
          }
        }
      }
    }
  } catch {
    /* The next poll retries a transient local connection failure. */
  }
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
