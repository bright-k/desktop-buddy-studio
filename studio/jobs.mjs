import { randomUUID } from "node:crypto";
import * as fs from "node:fs/promises";
import { join, basename } from "node:path";
import { run } from "./cli.mjs";
import { atomicJson, StudioError, requireId } from "./store.mjs";
const removable = new Set(["succeeded", "failed", "cancelled", "interrupted", "partial", "import_failed", "sync_failed"]);

export class BuildQueue {
  constructor(store, { templatePack, development = false, target } = {}) {
    this.store = store;
    this.templatePack = templatePack;
    this.development = development;
    this.target =
      target ?? (process.platform === "win32" ? "windows-x64" : "macos-arm64");
    this.queue = [];
    this.active = null;
  }
  async init() {
    for (const project of await this.store.list()) {
      for (const job of await this.list(project.id)) {
        if (job.provider !== "github" && ["queued", "running", "cancelling"].includes(job.status)) {
          job.status = "interrupted";
          job.error = "Studio가 종료되어 중단됐습니다. 다시 빌드하세요.";
          job.finishedAt = new Date().toISOString();
          await this.persist(job);
        }
      }
    }
  }
  file(job) {
    return join(
      this.store.path(job.projectId),
      "jobs",
      `${requireId(job.id)}.json`,
    );
  }
  async persist(job) {
    await atomicJson(this.file(job), job);
  }
  async list(projectId) {
    await this.store.metadata(projectId);
    const dir = join(this.store.path(projectId), "jobs");
    const names = await fs.readdir(dir).catch(() => []);
    return (
      await Promise.all(
        names
          .filter((n) => /^[a-f0-9-]{36}\.json$/.test(n))
          .map((n) => fs.readFile(join(dir, n), "utf8").then(JSON.parse).catch((error) => {
            if (error.code !== "ENOENT") throw error;
            return null;
          })),
      )
    ).filter(Boolean).sort((a, b) => b.createdAt.localeCompare(a.createdAt));
  }
  async removeHistory(projectId, id, removeFiles = false) {
    return this.store.exclusive(projectId, async () => {
      const meta = await this.store.metadata(projectId);
      const project = this.store.path(projectId);
      const path = this.file({ projectId, id });
      const job = await fs.readFile(path, "utf8").then(JSON.parse).catch((error) => {
        if (error.code === "ENOENT") throw new StudioError("빌드 기록을 찾을 수 없습니다.", 404);
        throw error;
      });
      if (job.projectId !== projectId || job.id !== id)
        throw new StudioError("빌드 기록의 프로젝트 정보가 일치하지 않습니다.", 409);
      if (!removable.has(job.status))
        throw new StudioError("진행 중인 빌드는 종료된 뒤 기록을 삭제할 수 있습니다.", 409);
      // Preserve the executable name used by an installed build after its
      // history and report have been removed.
      if (!meta.builtSlug && (job.status === "succeeded" || job.status === "partial" || job.result?.jobId)) {
        const definition = JSON.parse(await fs.readFile(join(project, "revisions", requireId(job.revision), "buddy.project.json"), "utf8"));
        if (definition.product.identifier === meta.identifier) {
          meta.builtSlug = definition.product.slug;
          await atomicJson(join(project, "project.json"), meta);
        }
      }
      if (removeFiles) await this.removeBuildFiles(projectId, job);
      await fs.unlink(path);
      return { deleted: true, builtSlug: meta.builtSlug };
    });
  }
  async removeBuildFiles(projectId, job) {
    const root = join(this.store.path(projectId), "builds");
    const entries = await fs.readdir(root, { withFileTypes: true }).catch((error) => {
      if (error.code === "ENOENT") return null;
      throw error;
    });
    if (!entries) return;
    const expected = join(await fs.realpath(this.store.root), "projects", projectId, "builds");
    if (await fs.realpath(root) !== expected)
      throw new StudioError("프로젝트 밖의 빌드 파일은 삭제할 수 없습니다.", 403);
    const legacyId = job.logs?.match(/^job-id: ([a-f0-9-]{36})\r?$/m)?.[1];
    const builderId = job.provider === "github" ? null : job.result?.jobId ?? job.buildJobId ?? legacyId;
    if (builderId) requireId(builderId);
    const directories = entries.filter((entry) => job.provider === "github"
      ? entry.name === `actions-${job.id}`
      : !!builderId && entry.name.endsWith(`-${builderId}`));
    if (directories.length > 1 || directories.some((entry) => !entry.isDirectory()))
      throw new StudioError("빌드 결과 폴더를 안전하게 확인할 수 없습니다.", 409);
    for (const entry of directories)
      await fs.rm(join(root, entry.name), { recursive: true, force: true, maxRetries: 3 });
    if (builderId) {
      for (const name of [`build-${builderId}.log`, `failed-${builderId}.json`])
        await fs.rm(join(root, name), { force: true });
    }
  }
  async enqueue(projectId, revision, development = this.development) {
    return this.store.exclusive("@build-queue", () =>
      this.enqueueUnique(projectId, revision, development),
    );
  }
  async enqueueUnique(projectId, revision, development) {
    const meta = await this.store.metadata(projectId);
    if (meta.archived) throw new StudioError("보관된 프로젝트를 복원하세요.");
    if (revision !== meta.revision)
      throw new StudioError("저장한 최신 구성에서 빌드하세요.", 409);
    const existing = [
      ...(this.active ? [this.active.job] : []),
      ...this.queue,
    ].find(
      (j) =>
        j.projectId === projectId &&
        j.revision === revision &&
        j.development === !!development &&
        ["queued", "running", "cancelling"].includes(j.status),
    );
    if (existing) return existing;
    if (this.queue.length >= 10)
      throw new StudioError("빌드 대기열이 가득 찼습니다.");
    const job = {
      id: randomUUID(),
      projectId,
      revision,
      status: "queued",
      createdAt: new Date().toISOString(),
      development: !!development,
      target: this.target,
      logs: "",
    };
    await this.persist(job);
    this.queue.push(job);
    void this.pump();
    return job;
  }
  async cancel(projectId, id) {
    const active = this.active;
    if (active?.job.id === id && active.job.projectId === projectId) {
      if (active.job.status === "cancelling") return active.job;
      if (active.job.status !== "running")
        throw new StudioError("이미 종료된 빌드입니다.");
      active.job.status = "cancelling";
      active.controller.abort();
      await active.flush();
      return active.job;
    }
    const job = this.queue.find((j) => j.id === id && j.projectId === projectId);
    if (!job) throw new StudioError("실행 중이거나 대기 중인 빌드가 없습니다.");
    this.queue = this.queue.filter((j) => j !== job);
    job.status = "cancelled";
    job.finishedAt = new Date().toISOString();
    await this.persist(job);
    return job;
  }
  async pump() {
    if (this.active || !this.queue.length) return;
    const job = this.queue.shift(),
      controller = new AbortController();
    let write = Promise.resolve(),
      lastFlush = 0;
    const flush = () => {
      const snapshot = structuredClone(job);
      // An earlier progress write can fail. Still attempt later states in order.
      write = write.catch(() => {}).then(() => this.persist(snapshot));
      return write;
    };
    this.active = { job, controller, flush };
    try {
      job.status = "running";
      job.startedAt = new Date().toISOString();
      job.buildJobId = job.id;
      await flush();
      const path = join(
        this.store.path(job.projectId),
        "revisions",
        job.revision,
      );
      const output = join(this.store.path(job.projectId), "builds");
      const result = JSON.parse(
        await run(
          this.store.cli,
          [
            "build",
            path,
            "--template-pack",
            this.templatePack,
            "--output",
            output,
            "--job-id",
            job.buildJobId,
            "--target",
            this.target,
            ...(job.development ? ["--development"] : []),
          ],
          {
            signal: controller.signal,
            onLog: (chunk) => {
              job.logs = (job.logs + chunk).slice(-128 * 1024);
              if (Date.now() - lastFlush > 500) {
                lastFlush = Date.now();
                void flush().catch(() => {});
              }
            },
          },
        ),
      );
      job.result = result;
      // A development .app is a directory; provide an ordinary downloadable ZIP.
      if (job.target === "macos-arm64") {
        const [app] = (await fs.readdir(result.directory)).filter((n) =>
          n.endsWith(".app"),
        );
        if (!app) throw new StudioError("macOS 앱 번들을 찾을 수 없습니다.");
        const { version, platformIdentity } = JSON.parse(
          await fs.readFile(join(result.directory, "build-report.json"), "utf8"),
        );
        const zip = `${platformIdentity.binaryName}_${version}_macos-arm64.app.zip`;
        const zipPath = join(result.directory, zip);
        await run(
          "/usr/bin/ditto",
          ["-c", "-k", "--sequesterRsrc", "--keepParent", join(result.directory, app), zipPath],
          { signal: controller.signal },
        );
        job.downloads = [{
          name: zip,
          path: `${basename(result.directory)}/${zip}`,
          bytes: (await fs.stat(zipPath)).size,
        }];
      }
      job.downloads = [
        ...(job.downloads ?? []),
        ...result.artifacts
          .filter((a) => /\.(dmg|exe|msi|zip)$/.test(a.path) || /^(?:Windows )?Installation Guide\.(pdf|html|png|jpe?g)$/.test(a.path))
          .map((a) => ({
            name: basename(a.path),
            path: `${basename(result.directory)}/${a.path}`,
            bytes: a.bytes,
          })),
        ...["build-report.json", "build.log", "INSTALL.txt", "SHA256SUMS"].map(
          (name) => ({ name, path: `${basename(result.directory)}/${name}` }),
        ),
      ];
      job.status = controller.signal.aborted ? "cancelled" : "succeeded";
    } catch (error) {
      job.status = controller.signal.aborted ? "cancelled" : "failed";
      job.error = error.message;
    } finally {
      job.finishedAt = new Date().toISOString();
      await flush().catch((e) =>
        console.error("Job persistence failed:", e.message),
      );
      this.active = null;
      void this.pump();
    }
  }
  async close() {
    if (this.active?.job.status === "running")
      await this.cancel(this.active.job.projectId, this.active.job.id);
    for (const job of [...this.queue]) await this.cancel(job.projectId, job.id);
    while (this.active) await new Promise((resolve) => setTimeout(resolve, 50));
  }
}
