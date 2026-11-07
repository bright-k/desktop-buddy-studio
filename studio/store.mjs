import { randomUUID, createHash } from "node:crypto";
import fs from "node:fs/promises";
import { join, resolve, relative, sep, dirname, extname } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { run } from "./cli.mjs";
export class StudioError extends Error {
  constructor(message, status = 400) {
    super(message);
    this.status = status;
  }
}
const uuidPattern =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
export const requireId = (value) => {
  if (!uuidPattern.test(value ?? ""))
    throw new StudioError("잘못된 프로젝트 ID입니다.");
  return value;
};
export const isPackageId = (value) =>
  typeof value === "string" &&
  value.length <= 128 &&
  /^[a-z][a-z0-9]*(?:\.[a-z][a-z0-9]*){2,}$/.test(value) &&
  !/^(con|prn|aux|nul|com[1-9]|lpt[1-9])\./.test(value);
export const requirePackageId = (value) => {
  if (!isPackageId(value))
    throw new StudioError(
      "패키지 ID는 com.example.typingcat 형식의 소문자 영문·숫자 이름이어야 합니다.",
    );
  return value;
};
export function suggestedPackageId(name) {
  let part =
    String(name)
      .toLowerCase()
      .replace(/[^a-z0-9]/g, "")
      .slice(0, 48) || "buddy";
  if (!/^[a-z]/.test(part)) part = "buddy" + part;
  return "io.desktopbuddy.app." + part;
}
function suggestedSlug(name, packageId) {
  const slug = String(name)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "")
    .slice(0, 60)
    .replace(/-$/, "");
  const candidate = slug.length >= 2 ? slug : packageId.split(".").at(-1);
  return candidate.length < 2 ||
    /^(con|prn|aux|nul|com[1-9]|lpt[1-9])$/.test(candidate)
    ? "buddy-" + candidate
    : candidate;
}
export async function atomicJson(path, value) {
  const temp = `${path}.${randomUUID()}.tmp`;
  await fs.mkdir(dirname(path), { recursive: true });
  try {
    await fs.writeFile(temp, JSON.stringify(value, null, 2) + "\n", {
      flag: "wx",
    });
    // A reader can temporarily prevent replacement on Windows. Leave the
    // previous JSON in place while waiting for the file to become available.
    for (let wait = 10; ; wait *= 2) {
      try {
        await fs.rename(temp, path);
        break;
      } catch (error) {
        if (wait > 320 || !["EPERM", "EBUSY", "EACCES"].includes(error.code))
          throw error;
        await delay(wait);
      }
    }
  } finally {
    await fs.rm(temp, { force: true });
  }
}
export async function scopedFile(root, name) {
  if (
    !name ||
    name.includes("\\") ||
    name.split("/").some((p) => !p || p === "." || p === "..")
  )
    throw new StudioError("잘못된 파일 경로입니다.");
  const base = await fs.realpath(root),
    path = await fs.realpath(join(root, name)).catch(() => {
      throw new StudioError("파일이 없습니다.", 404);
    });
  if (!path.startsWith(base + sep) || !(await fs.stat(path)).isFile())
    throw new StudioError("프로젝트 밖의 파일은 열 수 없습니다.", 403);
  return path;
}
export class ProjectStore {
  constructor({ dataRoot, cli }) {
    this.root = resolve(dataRoot);
    this.cli = cli;
    this.locks = new Map();
    this.aliases = new Map();
  }
  path(id) {
    return join(this.root, "projects", this.resolveId(id));
  }
  resolveId(id) {
    if (this.aliases.has(id)) return this.aliases.get(id);
    if (uuidPattern.test(id ?? ""))
      throw new StudioError("Project not found.", 404);
    return requirePackageId(id);
  }
  async init({ migrate = true } = {}) {
    await fs.mkdir(join(this.root, "projects"), { recursive: true });
    await fs.mkdir(join(this.root, ".staging"), { recursive: true });
    if (migrate) await this.migratePackageDirectories();
  }
  async migratePackageDirectories() {
    const root = join(this.root, "projects"),
      plans = [],
      packages = new Set();
    for (const entry of await fs.readdir(root, { withFileTypes: true })) {
      if (entry.isSymbolicLink())
        throw new StudioError("Project directories cannot be symlinks.");
      if (!entry.isDirectory()) continue;
      const source = join(root, entry.name),
        meta = JSON.parse(
          await fs.readFile(join(source, "project.json"), "utf8"),
        );
      const project = JSON.parse(
        await fs.readFile(
          join(
            source,
            "revisions",
            requireId(meta.revision),
            "buddy.project.json",
          ),
          "utf8",
        ),
      );
      const packageId = requirePackageId(project.product.identifier);
      if (
        meta.identifier !== packageId ||
        (!uuidPattern.test(entry.name) && entry.name !== packageId) ||
        (!uuidPattern.test(meta.id) && meta.id !== packageId)
      )
        throw new StudioError(
          "Project folder and package identity differ: " + entry.name,
        );
      if (packages.has(packageId))
        throw new StudioError(
          "Duplicate package ID; no folders were overwritten: " + packageId,
          409,
        );
      packages.add(packageId);
      const target = join(root, packageId);
      if (
        source !== target &&
        (await fs.lstat(target).then(
          () => true,
          () => false,
        ))
      )
        throw new StudioError(
          "Package directory already exists: " + packageId,
          409,
        );
      const journal = JSON.parse(
        await fs
          .readFile(join(source, "package-id-migration.json"), "utf8")
          .catch(() => "null"),
      );
      plans.push({
        source,
        target,
        meta,
        packageId,
        fromId: journal?.fromId ?? meta.id,
        needsMigration:
          entry.name !== packageId ||
          meta.id !== packageId ||
          journal?.status === "pending",
      });
    }
    // Check every destination before touching any existing folder. The journal
    // travels with an atomic rename and resumes if interruption occurs mid-way.
    for (const p of plans) {
      const { source, target, meta, packageId, fromId } = p;
      if (p.needsMigration) {
        await fs
          .writeFile(
            join(source, "project.before-package-id.json"),
            JSON.stringify(meta, null, 2) + "\n",
            { flag: "wx" },
          )
          .catch((e) => {
            if (e.code !== "EEXIST") throw e;
          });
        await atomicJson(join(source, "package-id-migration.json"), {
          version: 1,
          fromId,
          toId: packageId,
          status: "pending",
        });
        if (source !== target) await fs.rename(source, target);
      }
      const legacyIds = [
        ...new Set([
          ...(meta.legacyIds ?? []),
          ...(uuidPattern.test(meta.id) ? [meta.id] : []),
          ...(uuidPattern.test(fromId) ? [fromId] : []),
        ]),
      ];
      const updated = {
        ...meta,
        id: packageId,
        packageId,
        storageVersion: 2,
        legacyIds,
      };
      if (p.needsMigration || meta.storageVersion !== 2)
        await atomicJson(join(target, "project.json"), updated);
      for (const alias of legacyIds) {
        if (this.aliases.has(alias) && this.aliases.get(alias) !== packageId)
          throw new StudioError("Duplicate legacy project ID.");
        this.aliases.set(alias, packageId);
      }
      if (p.needsMigration) {
        const jobs = join(target, "jobs");
        for (const name of await fs.readdir(jobs).catch(() => [])) {
          if (!/^[a-f0-9-]{36}\.json$/.test(name)) continue;
          const path = join(jobs, name),
            job = JSON.parse(await fs.readFile(path, "utf8"));
          const oldRoot = join(root, fromId),
            directory = job.result?.directory;
          if (directory?.startsWith(oldRoot + sep)) {
            job.result.originalDirectory ??= directory;
            job.result.directory = target + directory.slice(oldRoot.length);
          }
          job.projectId = packageId;
          await atomicJson(path, job);
        }
        await atomicJson(join(target, "package-id-migration.json"), {
          version: 1,
          fromId,
          toId: packageId,
          status: "complete",
          completedAt: new Date().toISOString(),
        });
      }
    }
  }
  async exclusive(id, work) {
    const previous = this.locks.get(id) ?? Promise.resolve();
    const next = previous.catch(() => {}).then(work);
    this.locks.set(id, next);
    try {
      return await next;
    } finally {
      if (this.locks.get(id) === next) this.locks.delete(id);
    }
  }
  async metadata(id) {
    return JSON.parse(
      await fs
        .readFile(join(this.path(id), "project.json"), "utf8")
        .catch(() => {
          throw new StudioError("프로젝트가 없습니다.", 404);
        }),
    );
  }
  async list() {
    const projects = [];
    for (const id of await fs.readdir(join(this.root, "projects"))) {
      if (!isPackageId(id)) continue;
      projects.push(await this.metadata(id));
    }
    return projects.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
  }
  async inspectPath(path, output) {
    return JSON.parse(
      await run(this.cli, [
        "inspect",
        path,
        ...(output ? ["--output", output] : []),
      ]),
    );
  }
  async currentPath(id) {
    const meta = await this.metadata(id);
    return join(this.path(id), "revisions", requireId(meta.revision));
  }
  async load(id) {
    return this.loadRevision(id, await this.metadata(id));
  }
  async loadRevision(
    id,
    meta,
    path = join(this.path(id), "revisions", requireId(meta.revision)),
  ) {
    const info = await this.inspectPath(
      path,
      join(this.path(id), "previews", meta.revision),
    );
    const library = JSON.parse(
      await fs
        .readFile(join(this.path(id), "resources", "index.json"), "utf8")
        .catch(() => "{}"),
    );
    const license = await fs.readFile(
      join(path, "character", "LICENSE.txt"),
      "utf8",
    );
    const storedDraft = await this.readDraft(id);
    const draft =
      storedDraft?.baseRevision === meta.revision ? storedDraft : null;
    return { ...info, meta, library, license, draft };
  }
  async readDraft(id) {
    return JSON.parse(
      await fs
        .readFile(join(this.path(id), "draft.json"), "utf8")
        .catch((e) => {
          if (e.code === "ENOENT") return "null";
          throw e;
        }),
    );
  }
  async saveDraft(id, payload) {
    return this.exclusive(id, async () => {
      const meta = await this.metadata(id);
      const stored = await this.readDraft(id),
        current = stored?.baseRevision === meta.revision ? stored : null;
      if (meta.archived)
        throw new StudioError("보관된 프로젝트는 복원 후 수정하세요.");
      if (payload.revision !== meta.revision)
        throw new StudioError(
          "다른 창에서 구성을 저장했습니다. 현재 편집을 JSON으로 보관한 뒤 프로젝트를 다시 여세요.",
          409,
        );
      if ((current?.draftRevision ?? null) !== (payload.draftRevision ?? null))
        throw new StudioError(
          "다른 창에서 임시 편집을 변경했습니다. 현재 편집을 JSON으로 보관한 뒤 프로젝트를 다시 여세요.",
          409,
        );
      const project = payload.project,
        character = payload.character;
      const source = JSON.parse(
        await fs.readFile(
          join(await this.currentPath(id), "buddy.project.json"),
          "utf8",
        ),
      );
      if (
        !project ||
        !character ||
        project.projectId !== source.projectId ||
        project.product?.identifier !== meta.identifier ||
        typeof character.animations !== "object" ||
        !character.sheet ||
        !character.anchor ||
        typeof payload.license !== "string"
      )
        throw new StudioError("임시 편집 문서가 올바르지 않습니다.");
      const draft = {
        baseRevision: meta.revision,
        draftRevision: randomUUID(),
        project,
        character,
        license: payload.license,
        updatedAt: new Date().toISOString(),
      };
      if (Buffer.byteLength(JSON.stringify(draft)) > 2 * 1024 * 1024)
        throw new StudioError("임시 편집은 2 MiB 이하여야 합니다.");
      await atomicJson(join(this.path(id), "draft.json"), draft);
      return draft;
    });
  }
  async archiveDraft(id, reason) {
    const draft = await this.readDraft(id);
    if (!draft) return;
    const dir = join(this.path(id), "drafts");
    await fs.mkdir(dir, { recursive: true });
    const backup = join(dir, `${reason}-${requireId(draft.draftRevision)}.json`);
    await fs.rename(join(this.path(id), "draft.json"), backup);
    return backup;
  }
  async discardDraft(id, draftRevision) {
    return this.exclusive(id, async () => {
      const meta = await this.metadata(id),
        current = await this.readDraft(id);
      if (
        current?.baseRevision === meta.revision &&
        current.draftRevision !== draftRevision
      )
        throw new StudioError("다른 창에서 임시 편집을 변경했습니다.", 409);
      await this.archiveDraft(id, "discarded");
      return { discarded: true };
    });
  }
  async create(options = {}) {
    return this.exclusive("@projects", () => this.createProject(options));
  }
  async createProject({
    name = "My Buddy",
    template = "typing-cat",
    packageId = suggestedPackageId(name),
  } = {}) {
    const id = requirePackageId(packageId);
    if (
      await fs.lstat(this.path(id)).then(
        () => true,
        () => false,
      )
    )
      throw new StudioError(
        "같은 패키지 ID의 프로젝트가 있습니다. 다른 패키지 ID를 입력하세요.",
        409,
      );
    if (!["typing-cat", "click-mascot"].includes(template))
      throw new StudioError("지원하지 않는 템플릿입니다.");
    const revision = randomUUID(),
      staging = join(this.root, ".staging", randomUUID()),
      destination = this.path(id);
    try {
      await run(this.cli, [
        "new",
        staging,
        "--name",
        name,
        "--template",
        template,
      ]);
      const info = await this.inspectPath(staging);
      const project = info.project;
      project.product.slug = suggestedSlug(name, id);
      project.product.identifier = id;
      await atomicJson(join(staging, "buddy.project.json"), project);
      const now = new Date().toISOString();
      const meta = {
        id,
        packageId: id,
        storageVersion: 2,
        legacyIds: [],
        revision,
        name: project.product.name,
        slug: project.product.slug,
        identifier: project.product.identifier,
        createdAt: now,
        updatedAt: now,
        archived: false,
      };
      await fs.mkdir(join(destination, "revisions"), { recursive: true });
      await fs.rename(staging, join(destination, "revisions", revision));
      await atomicJson(join(destination, "project.json"), meta);
      return this.load(id);
    } finally {
      await fs.rm(staging, { recursive: true, force: true });
    }
  }
  async stage(id, payload) {
    const base = this.path(id),
      source = await this.currentPath(id),
      revision = randomUUID();
    const path = join(this.root, ".staging", revision);
    const { project, character, license } = payload;
    const current = await this.inspectPath(source);
    if (
      project.projectId !== current.project.projectId ||
      project.product.identifier !== current.project.product.identifier
    )
      throw new StudioError(
        "프로젝트 ID와 앱 식별자는 생성 후 변경할 수 없습니다. 복제를 사용하세요.",
      );
    if (
      project.character !== "character/manifest.json" ||
      project.product.icon !== "assets/app-icon.png" ||
      character.image !== "sprite.png" ||
      character.license !== "LICENSE.txt"
    )
      throw new StudioError(
        "기본 파일 경로를 변경할 수 없습니다. 리소스를 추가하고 애니메이션에서 선택하세요.",
      );
    try {
      await fs.cp(source, path, { recursive: true, dereference: false });
      await fs.rm(join(path, "character", "resources"), {
        recursive: true,
        force: true,
      });
      await fs.mkdir(join(path, "character", "resources"), { recursive: true });
      for (const resource of Object.values(character.resources ?? {})) {
        if (
          !/^resources\/[a-f0-9]{64}\.(png|jpg|jpeg|gif|webp|bmp)$/.test(
            resource.image,
          )
        )
          throw new StudioError("업로드한 프로젝트 리소스를 선택하세요.");
        const name = resource.image.slice("resources/".length);
        await fs.copyFile(
          await scopedFile(join(base, "resources"), name),
          join(path, "character", resource.image),
        );
      }
      if (payload.icon) {
        if (!/^[a-f0-9]{64}\.png$/.test(payload.icon))
          throw new StudioError("아이콘은 PNG여야 합니다.");
        await fs.copyFile(
          await scopedFile(join(base, "resources"), payload.icon),
          join(path, "assets", "app-icon.png"),
        );
      }
      await atomicJson(join(path, "buddy.project.json"), project);
      await atomicJson(join(path, "character", "manifest.json"), character);
      if (typeof license === "string")
        await fs.writeFile(join(path, "character", "LICENSE.txt"), license);
      const info = await this.inspectPath(path);
      return { path, revision, info };
    } catch (error) {
      await fs.rm(path, { recursive: true, force: true });
      throw error;
    }
  }
  async validate(id, payload) {
    const staged = await this.stage(id, payload);
    try {
      return { valid: true, contentHash: staged.info.contentHash };
    } finally {
      await fs.rm(staged.path, { recursive: true, force: true });
    }
  }
  async save(id, payload) {
    return this.exclusive(id, async () => {
      const meta = await this.metadata(id);
      if (payload.revision !== meta.revision)
        throw new StudioError(
          "다른 창에서 수정되었습니다. 현재 내용을 복사한 뒤 프로젝트를 다시 여세요.",
          409,
        );
      if (meta.archived)
        throw new StudioError("보관된 프로젝트는 복원 후 수정하세요.");
      const draft = await this.readDraft(id);
      if (
        draft?.baseRevision === meta.revision &&
        draft.draftRevision !== payload.draftRevision
      )
        throw new StudioError(
          "다른 창에서 임시 편집을 변경했습니다. 현재 편집을 JSON으로 보관한 뒤 프로젝트를 다시 여세요.",
          409,
        );
      const staged = await this.stage(id, payload);
      const revisionPath = join(this.path(id), "revisions", staged.revision);
      const nextMeta = {
        ...meta,
        revision: staged.revision,
        name: payload.project.product.name,
        slug: payload.project.product.slug,
        updatedAt: new Date().toISOString(),
      };
      let view;
      let moved = false;
      try {
        view = await this.loadRevision(id, nextMeta, staged.path);
        await fs.rename(staged.path, revisionPath);
        moved = true;
        await atomicJson(join(this.path(id), "project.json"), nextMeta);
      } catch (error) {
        await Promise.allSettled([
          fs.rm(moved ? revisionPath : staged.path, { recursive: true, force: true }),
          fs.rm(join(this.path(id), "previews", staged.revision), { recursive: true, force: true }),
        ]);
        throw error;
      }
      await this.archiveDraft(id, "saved").catch((error) =>
        console.error("Draft archive failed:", error.message),
      );
      return view;
    });
  }
  async archive(id, archived) {
    return this.exclusive(id, async () => {
      const meta = await this.metadata(id);
      await atomicJson(join(this.path(id), "project.json"), {
        ...meta,
        archived: !!archived,
        updatedAt: new Date().toISOString(),
      });
      return this.metadata(id);
    });
  }
  async clone(id) {
    return this.exclusive(id, async () => {
      const original = await this.load(id);
      const base = original.project.product.identifier;
      let packageId =
        base.length < 122
          ? base + ".copy"
          : suggestedPackageId(original.project.product.name + "Copy");
      let suffix = 2;
      const first = packageId;
      while (
        await fs.lstat(this.path(packageId)).then(
          () => true,
          () => false,
        )
      )
        packageId = first + suffix++;
      const fresh = await this.create({
        name: `${original.project.product.name.slice(0, 65)} Copy`,
        packageId,
      });
      const target = this.path(fresh.meta.id);
      const library = join(this.path(id), "resources");
      await fs
        .cp(library, join(target, "resources"), { recursive: true })
        .catch((error) => {
          if (error.code !== "ENOENT") throw error;
        });
      const project = {
        ...original.project,
        projectId: fresh.project.projectId,
        product: {
          ...original.project.product,
          name: fresh.project.product.name,
          slug: fresh.project.product.slug,
          identifier: fresh.project.product.identifier,
        },
      };
      // Preserve the source sheet and icon before saving the copied definitions.
      const current = await this.currentPath(fresh.meta.id),
        source = await this.currentPath(id);
      await fs.copyFile(
        join(source, "character", "sprite.png"),
        join(current, "character", "sprite.png"),
      );
      await fs.copyFile(
        join(source, "assets", "app-icon.png"),
        join(current, "assets", "app-icon.png"),
      );
      return this.save(fresh.meta.id, {
        revision: fresh.meta.revision,
        project,
        character: original.character,
        license: original.license,
      });
    });
  }
  async upload(id, bytes, filename) {
    return this.exclusive(id, async () => {
      await this.metadata(id);
      const extension = extname(filename).slice(1).toLowerCase();
      if (!["png", "jpg", "jpeg", "gif", "webp", "bmp"].includes(extension))
        throw new StudioError(
          "PNG · JPG · GIF · WebP · BMP 파일을 선택하세요.",
        );
      if (!bytes.length || bytes.length > 16 * 1024 * 1024)
        throw new StudioError("리소스는 16 MiB 이하여야 합니다.");
      const probe = join(this.root, ".staging", `${randomUUID()}.${extension}`);
      let media;
      try {
        await fs.writeFile(probe, bytes, { flag: "wx" });
        media = JSON.parse(await run(this.cli, ["media-info", probe]));
      } finally {
        await fs.rm(probe, { force: true });
      }
      const name =
        createHash("sha256").update(bytes).digest("hex") + "." + extension;
      const root = join(this.path(id), "resources");
      await fs.mkdir(root, { recursive: true });
      const index = JSON.parse(
        await fs.readFile(join(root, "index.json"), "utf8").catch(() => "{}"),
      );
      if (
        !index[name] &&
        Object.values(index).reduce((n, a) => n + a.bytes, 0) + bytes.length >
          256 * 1024 * 1024
      )
        throw new StudioError("프로젝트 리소스 보관함 한도는 256 MiB입니다.");
      await fs.writeFile(join(root, name), bytes, { flag: "wx" }).catch((e) => {
        if (e.code !== "EEXIST") throw e;
      });
      index[name] = {
        name,
        originalName: filename.slice(0, 200),
        ...media,
        bytes: bytes.length,
        createdAt: new Date().toISOString(),
      };
      await atomicJson(join(root, "index.json"), index);
      return index[name];
    });
  }
  async revisions(id) {
    const base = join(this.path(id), "revisions");
    const records = [];
    for (const revision of await fs.readdir(base)) {
      if (!uuidPattern.test(revision)) continue;
      const project = JSON.parse(
        await fs.readFile(join(base, revision, "buddy.project.json"), "utf8"),
      );
      records.push({
        revision,
        name: project.product.name,
        version: project.product.version,
        updatedAt: (await fs.stat(join(base, revision))).mtime.toISOString(),
      });
    }
    return records.sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
  }
  async restore(id, revision, expectedRevision, draftRevision) {
    return this.exclusive(id, async () => {
      const meta = await this.metadata(id);
      if (meta.revision !== expectedRevision)
        throw new StudioError("다른 창에서 수정되었습니다.", 409);
      if (meta.archived)
        throw new StudioError("보관된 프로젝트는 복원 후 수정하세요.");
      const draft = await this.readDraft(id);
      if (draft?.baseRevision === meta.revision && draft.draftRevision !== draftRevision)
        throw new StudioError(
          "다른 창에서 임시 편집을 변경했습니다. 최신 임시 편집을 확인한 뒤 복원하세요.",
          409,
        );
      const view = await this.loadRevision(id, { ...meta, revision });
      const nextMeta = {
        ...view.meta,
        name: view.project.product.name,
        slug: view.project.product.slug,
        updatedAt: new Date().toISOString(),
      };
      const backup = await this.archiveDraft(id, "restored");
      try {
        await atomicJson(join(this.path(id), "project.json"), nextMeta);
      } catch (error) {
        if (backup) await fs.rename(backup, join(this.path(id), "draft.json"));
        throw error;
      }
      return { ...view, meta: nextMeta, draft: null };
    });
  }
}
