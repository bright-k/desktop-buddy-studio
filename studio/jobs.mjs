import { randomUUID } from "node:crypto";
import * as fs from "node:fs/promises";
import { join, basename } from "node:path";
import { run } from "./cli.mjs";
import { atomicJson, StudioError, requireId } from "./store.mjs";
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
          .map((n) => fs.readFile(join(dir, n), "utf8").then(JSON.parse)),
      )
    ).sort((a, b) => b.createdAt.localeCompare(a.createdAt));
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
    const job =
      this.active?.job.id === id && this.active.job.projectId === projectId
        ? this.active.job
        : this.queue.find((j) => j.id === id && j.projectId === projectId);
    if (!job) throw new StudioError("실행 중이거나 대기 중인 빌드가 없습니다.");
    if (job.status === "queued") {
      this.queue = this.queue.filter((j) => j !== job);
      job.status = "cancelled";
      job.finishedAt = new Date().toISOString();
    } else {
      job.status = "cancelling";
      this.active.controller.abort();
    }
    await this.persist(job);
    return job;
  }
  async pump() {
    if (this.active || !this.queue.length) return;
    const job = this.queue.shift(),
      controller = new AbortController();
    this.active = { job, controller };
    let write = Promise.resolve(),
      lastFlush = 0;
    const flush = () => {
      const snapshot = structuredClone(job);
      write = write.then(() => this.persist(snapshot));
      return write;
    };
    try {
      job.status = "running";
      job.startedAt = new Date().toISOString();
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
      if (process.platform === "darwin") {
        const apps = (await fs.readdir(result.directory)).filter((n) =>
          n.endsWith(".app"),
        );
        for (const app of apps) {
          const zip = `${app}.zip`;
          await run(
            "/usr/bin/ditto",
            [
              "-c",
              "-k",
              "--sequesterRsrc",
              "--keepParent",
              join(result.directory, app),
              join(result.directory, zip),
            ],
            { signal: controller.signal },
          );
          job.downloads = [
            ...(job.downloads ?? []),
            {
              name: zip,
              path: `${basename(result.directory)}/${zip}`,
              bytes: (await fs.stat(join(result.directory, zip))).size,
            },
          ];
        }
      }
      job.downloads = [
        ...(job.downloads ?? []),
        ...result.artifacts
          .filter((a) => /\.(dmg|exe|msi|zip)$/.test(a.path))
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
    if (this.active)
      await this.cancel(this.active.job.projectId, this.active.job.id);
    for (const job of [...this.queue]) await this.cancel(job.projectId, job.id);
    while (this.active) await new Promise((resolve) => setTimeout(resolve, 50));
  }
}
