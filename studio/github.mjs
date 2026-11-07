import * as fs from "node:fs/promises";
import { join } from "node:path";
import { randomUUID, createHash } from "node:crypto";
import { GitHubAPI, githubCredential, repositoryName } from "./github-api.mjs";
import { atomicJson, StudioError, scopedFile, requireId } from "./store.mjs";
import { run } from "./cli.mjs";
import { verifyArtifact } from "./github-artifacts.mjs";

const workflowPath = ".github/workflows/studio-build.yml";
const workflowName = "studio-build.yml";
const targets = ["macos-arm64", "windows-x64"];
const ongoing = new Set(["preparing", "queued", "running", "downloading", "awaiting_auth", "dispatch_unknown", "cancelling"]);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
const gitHash = (bytes) => createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
const now = () => new Date().toISOString();

export class ActionsBuilds {
  constructor(store, jobs, { templatePack, githubFetch, credentialProvider = githubCredential, pollInterval = 10000 } = {}) {
    this.store = store; this.jobs = jobs; this.templatePack = templatePack;
    this.githubFetch = githubFetch; this.credentialProvider = credentialProvider; this.pollInterval = pollInterval;
    this.profile = { repository: "bright-k/desktop-buddy-studio", connected: false, ready: false };
    this.configPath = join(store.root, "github.json"); this.closed = false;
  }
  async init() {
    const config = JSON.parse(await fs.readFile(this.configPath, "utf8").catch(() => "{}"));
    if (config.repository) this.profile.repository = repositoryName(config.repository);
    if (config.repository && config.credentialSource !== "session") {
      try { await this.connect({ repository: config.repository }); } catch (e) { this.profile.error = e.message; }
    }
    this.schedule();
  }
  status() { return { ...this.profile }; }
  async connect({ repository, token }) {
    repository = repositoryName(repository);
    const credential = token ? { token, source: "session" } : await this.credentialProvider();
    if (!credential?.token || typeof credential.token !== "string" || /[\r\n]/.test(credential.token) || credential.token.length > 4096)
      throw new StudioError("GitHub 인증이 없습니다. Git 인증을 준비하거나 아래 토큰을 입력하세요. 토큰은 이 Studio 실행 동안만 유지됩니다.", 401);
    const api = new GitHubAPI(credential.token, { fetcher: this.githubFetch });
    try {
      const [user, repo] = await Promise.all([api.request("/user"), api.request(`/repos/${repository}`)]);
      if (!repo.permissions?.push) throw new StudioError("연결할 저장소의 쓰기 권한이 필요합니다.", 403);
      const workflow = await this.workflowStatus(api, repository, repo.default_branch);
      this.api?.close(); this.api = api;
      this.profile = { repository, connected: true, login: user.login, private: !!repo.private,
        defaultBranch: repo.default_branch, credentialSource: credential.source, ...workflow };
      await atomicJson(this.configPath, { repository, credentialSource: credential.source });
      this.wake();
      return this.status();
    } catch (e) { api.close(); throw e; }
  }
  async workflowBytes() { return fs.readFile(await scopedFile(this.templatePack, workflowPath)); }
  async workflowStatus(api, repo, branch) {
    let existing;
    try { existing = await api.request(`/repos/${repo}/contents/${workflowPath}?ref=${encodeURIComponent(branch)}`); }
    catch (e) { if (e.githubStatus !== 404) throw e; }
    const expected = await this.workflowBytes();
    return { ready: !!existing && existing.encoding === "base64" && Buffer.from(existing.content, "base64").equals(expected),
      workflowPresent: !!existing };
  }
  async setup() {
    if (!this.api) throw new StudioError("GitHub 저장소를 먼저 연결하세요.", 401);
    return this.store.exclusive("@github-setup", async () => {
      const api = this.api, p = this.profile;
      let existing;
      try { existing = await api.request(`/repos/${p.repository}/contents/${workflowPath}?ref=${encodeURIComponent(p.defaultBranch)}`); }
      catch (e) { if (e.githubStatus !== 404) throw e; }
      const bytes = await this.workflowBytes();
      if (!existing || !Buffer.from(existing.content || "", "base64").equals(bytes)) {
        await api.request(`/repos/${p.repository}/contents/${workflowPath}`, { method: "PUT", body: {
          message: "Configure Desktop Buddy Studio multi-platform builds", branch: p.defaultBranch,
          content: bytes.toString("base64"), ...(existing ? { sha: existing.sha } : {}),
        } });
      }
      Object.assign(this.profile, await this.workflowStatus(api, p.repository, p.defaultBranch));
      return this.status();
    });
  }
  async disconnect() {
    this.api?.close(); this.api = null;
    this.profile = { repository: this.profile.repository, connected: false, ready: false };
    await atomicJson(this.configPath, { repository: this.profile.repository, credentialSource: "session" });
    return this.status();
  }
  async enqueue(projectId, revision, { acknowledgeUpload = false } = {}) {
    if (!this.api || !this.profile.ready) throw new StudioError("GitHub 연결과 워크플로 준비를 먼저 완료하세요.");
    if (acknowledgeUpload !== true) throw new StudioError("선택한 저장소로 프로젝트·리소스를 업로드하는 빌드 버튼을 사용하세요.");
    return this.store.exclusive("@github-enqueue", async () => {
      const meta = await this.store.metadata(projectId);
      if (meta.archived || revision !== meta.revision) throw new StudioError("최신 구성을 저장한 뒤 빌드하세요.", 409);
      const info = await this.store.inspectPath(await this.store.currentPath(projectId));
      if (targets.some((t) => !info.project.build.targets.includes(t))) throw new StudioError("프로젝트의 빌드 대상에 macOS arm64와 Windows x64가 모두 필요합니다.");
      const previous = (await this.jobs.list(projectId)).find((j) => j.provider === "github" && j.revision === revision && j.github.repository === this.profile.repository && ongoing.has(j.status));
      if (previous) return previous;
      const job = { id: randomUUID(), provider: "github", target: "macos-arm64+windows-x64", projectId, revision, contentHash: info.contentHash,
        status: "preparing", createdAt: now(), development: true, logs: "저장된 프로젝트와 고정 엔진을 준비합니다.\n",
        github: { repository: this.profile.repository }, platforms: Object.fromEntries(targets.map((t) => [t, { status: "queued" }])) };
      await this.jobs.persist(job); this.wake(); return job;
    });
  }
  async uploadTree(api, repo, files, base) {
    let nodes = [], bytes = 0, tree = base;
    const flush = async () => {
      if (!nodes.length) return;
      const value = await api.request(`/repos/${repo}/git/trees`, { method: "POST", body: { ...(tree ? { base_tree: tree } : {}), tree: nodes } });
      tree = value.sha; nodes = []; bytes = 0;
    };
    for (const [path, value] of files) {
      if (this.closed || api !== this.api) throw new Error("GitHub 연결이 중단됐습니다.");
      const text = value.toString("utf8"); let node;
      if (Buffer.from(text, "utf8").equals(value) && !text.includes("\0")) node = { path, mode: "100644", type: "blob", content: text };
      else {
        const blob = await api.request(`/repos/${repo}/git/blobs`, { method: "POST", body: { encoding: "base64", content: value.toString("base64") } });
        if (blob.sha !== gitHash(value)) throw new StudioError("GitHub 리소스 업로드 hash가 일치하지 않습니다.");
        node = { path, mode: "100644", type: "blob", sha: blob.sha };
      }
      const size = Buffer.byteLength(JSON.stringify(node));
      if (bytes + size > 5 * 1024 * 1024) await flush();
      nodes.push(node); bytes += size;
    }
    await flush(); return tree;
  }
  async engine(api, repo) {
    const verified = JSON.parse(await run(this.store.cli, ["verify-template-pack", this.templatePack]));
    const engineHash = verified.templateHash, branch = `studio-engine/${engineHash}`;
    const manifestBytes = await fs.readFile(join(this.templatePack, "pack-manifest.json"));
    if (hash(manifestBytes) !== engineHash) throw new StudioError("고정 엔진 manifest가 변경됐습니다.");
    const manifest = JSON.parse(manifestBytes), files = new Map([["pack-manifest.json", manifestBytes], [".gitattributes", Buffer.from("* -text\n")]]);
    for (const file of manifest.files) {
      const data = await fs.readFile(await scopedFile(this.templatePack, file.path));
      if (data.length !== file.size || hash(data) !== file.sha256) throw new StudioError("고정 엔진 파일이 변경됐습니다.");
      files.set(file.path, data);
    }
    try {
      const ref = await api.request(`/repos/${repo}/git/ref/heads/${branch}`);
      const commit = await api.request(`/repos/${repo}/git/commits/${ref.object.sha}`);
      const remote = await api.request(`/repos/${repo}/git/trees/${commit.tree.sha}?recursive=1`);
      const blobs = remote.tree.filter((n) => n.type === "blob");
      if (remote.truncated || blobs.length !== files.size || blobs.some((n) => !files.has(n.path) || n.mode !== "100644" || n.sha !== gitHash(files.get(n.path))))
        throw new StudioError("GitHub의 같은 이름 엔진 브랜치가 고정 팩과 다릅니다. 덮어쓰지 않았습니다.");
      return { engineHash, commit: ref.object.sha, tree: commit.tree.sha };
    } catch (e) { if (e.githubStatus !== 404) throw e; }
    const tree = await this.uploadTree(api, repo, files);
    const commit = await api.request(`/repos/${repo}/git/commits`, { method: "POST", body: { message: `Studio engine ${engineHash}`, tree, parents: [] } });
    await api.request(`/repos/${repo}/git/refs`, { method: "POST", body: { ref: `refs/heads/${branch}`, sha: commit.sha } });
    return { engineHash, commit: commit.sha, tree };
  }
  async prepare(job) {
    const api = this.api, repo = job.github.repository;
    const stage = await fs.mkdtemp(join(this.store.root, ".staging", "github-"));
    try {
      const source = join(stage, "project");
      const info = JSON.parse(await run(this.store.cli, ["snapshot", join(this.store.path(job.projectId), "revisions", requireId(job.revision)), "--output", source]));
      if (info.contentHash !== job.contentHash) throw new StudioError("빌드 요청의 저장 구성 hash가 변경됐습니다.");
      const engine = await this.engine(api, repo);
      job.github.engineHash = engine.engineHash;
      job.github.branch = `studio-build/${job.id}`;
      const files = new Map();
      const walk = async (dir, prefix = "") => {
        for (const entry of await fs.readdir(dir, { withFileTypes: true })) {
          const name = prefix + entry.name;
          if (entry.isDirectory()) await walk(join(dir, entry.name), name + "/");
          else if (entry.isFile()) files.set(`.studio/project/${name}`, await fs.readFile(await scopedFile(source, name)));
          else throw new StudioError("프로젝트에 일반 파일만 포함할 수 있습니다.");
        }
      };
      await walk(source);
      files.set(".studio/request.json", Buffer.from(JSON.stringify({ jobId: job.id, contentHash: job.contentHash, revision: job.revision, packageId: job.projectId, engineHash: engine.engineHash })));
      const tree = await this.uploadTree(api, repo, files, engine.tree);
      const commit = await api.request(`/repos/${repo}/git/commits`, { method: "POST", body: { message: `Studio build ${job.id}`, tree, parents: [engine.commit] } });
      try { await api.request(`/repos/${repo}/git/refs`, { method: "POST", body: { ref: `refs/heads/${job.github.branch}`, sha: commit.sha } }); }
      catch (e) {
        if (e.githubStatus !== 422) throw e;
        const ref = await api.request(`/repos/${repo}/git/ref/heads/${job.github.branch}`);
        const prior = await api.request(`/repos/${repo}/git/commits/${ref.object.sha}`);
        if (prior.tree.sha !== tree) throw new StudioError("동일 빌드 브랜치의 내용이 다릅니다. 덮어쓰지 않았습니다.");
        commit.sha = ref.object.sha;
      }
      job.github.headSha = commit.sha;
      job.logs += `고정 엔진 ${engine.engineHash.slice(0, 12)} · 프로젝트 ${job.contentHash.slice(0, 12)} 업로드 완료\n`;
      await this.jobs.persist(job);
    } finally { await fs.rm(stage, { recursive: true, force: true }); }
  }
  async dispatch(job) {
    // Persist uncertainty before the POST: a crash/timeout must never auto-dispatch twice.
    job.github.dispatchAttemptedAt = now(); job.status = "dispatch_unknown";
    await this.jobs.persist(job);
    const value = await this.api.request(`/repos/${job.github.repository}/actions/workflows/${workflowName}/dispatches`, { method: "POST", body: {
      ref: job.github.branch, return_run_details: true,
      inputs: { job_id: job.id, project_hash: job.contentHash, engine_hash: job.github.engineHash },
    } });
    if (value?.workflow_run_id) job.github.runId = value.workflow_run_id;
    job.status = "queued"; job.logs += "GitHub Actions에 두 플랫폼 빌드를 요청했습니다.\n";
    await this.jobs.persist(job);
  }
  async sync(job) {
    const api = this.api, base = `/repos/${job.github.repository}/actions`;
    if (!job.github.runId) {
      const list = await api.request(`${base}/workflows/${workflowName}/runs?event=workflow_dispatch&branch=${encodeURIComponent(job.github.branch)}&per_page=20`);
      const runs = list.workflow_runs.filter((r) => r.head_sha === job.github.headSha);
      if (runs.length > 1) throw new StudioError("동일 요청에 여러 Actions 실행이 있습니다. GitHub에서 실행을 확인하세요.");
      if (!runs.length) return;
      job.github.runId = runs[0].id;
    }
    const remote = await api.request(`${base}/runs/${job.github.runId}`);
    if (remote.head_sha !== job.github.headSha || remote.head_branch !== job.github.branch || remote.event !== "workflow_dispatch")
      throw new StudioError("Actions 실행과 업로드한 저장 버전이 일치하지 않습니다.");
    const attempt = remote.run_attempt || 1;
    if (job.github.runAttempt && job.github.runAttempt !== attempt) throw new StudioError("GitHub에서 재실행된 빌드입니다. 새 Studio 빌드를 시작하세요.");
    job.github.runAttempt = attempt;
    job.github.runUrl = `https://github.com/${job.github.repository}/actions/runs/${job.github.runId}`;
    if (job.cancelRequested && remote.status !== "completed") {
      if (!job.github.cancelSent) { await api.request(`${base}/runs/${job.github.runId}/cancel`, { method: "POST" }); job.github.cancelSent = true; }
      job.status = "cancelling";
    } else job.status = remote.status === "completed" ? "downloading" : remote.status === "queued" ? "queued" : "running";
    const native = await api.request(`${base}/runs/${job.github.runId}/jobs?filter=latest&per_page=100`);
    for (const target of targets) {
      const platform = native.jobs.find((j) => j.name === target);
      if (platform) Object.assign(job.platforms[target], { status: platform.status, conclusion: platform.conclusion,
        steps: (platform.steps || []).map((s) => ({ name: s.name, status: s.status, conclusion: s.conclusion })) });
    }
    job.logs = targets.map((t) => `${t}: ${job.platforms[t].conclusion || job.platforms[t].status}\n` + (job.platforms[t].steps || []).map((s) => `  ${s.conclusion || s.status} · ${s.name}`).join("\n")).join("\n\n");
    const artifacts = await api.request(`${base}/runs/${job.github.runId}/artifacts?per_page=100`);
    for (const target of targets) {
      if (job.platforms[target].imported) continue;
      const artifact = artifacts.artifacts.find((a) => a.name === `studio-${job.id}-${target}-a${attempt}`);
      if (artifact) {
        try {
          if (artifact.expired) throw new StudioError("GitHub 보관 기간이 끝난 결과물입니다. 새 빌드가 필요합니다.");
          const zip = await api.request(`${base}/artifacts/${artifact.id}/zip`, { binary: true, maxBytes: 256 * 1024 * 1024 });
          const verified = verifyArtifact(zip, artifact, job, target);
          const directory = join(this.store.path(job.projectId), "builds", `actions-${job.id}`);
          await fs.mkdir(directory, { recursive: true });
          await fs.writeFile(join(directory, verified.archiveName + ".tmp"), verified.archive);
          await fs.rename(join(directory, verified.archiveName + ".tmp"), join(directory, verified.archiveName));
          await fs.writeFile(join(directory, verified.receiptName), verified.receipt);
          await atomicJson(join(directory, `report-${target}.json`), verified.report);
          job.downloads = [...(job.downloads || []), ...[verified.archiveName, verified.receiptName, `report-${target}.json`].map((name) => ({ name, path: `actions-${job.id}/${name}` }))];
          job.platforms[target].imported = true; delete job.platforms[target].importError;
          job.result = { directory };
        } catch (e) { job.platforms[target].importError = e.message; }
      }
    }
    if (remote.status === "completed") {
      const complete = targets.filter((t) => job.platforms[t].imported).length;
      job.remoteConclusion = remote.conclusion;
      if (complete === targets.length && remote.conclusion === "success") job.status = "succeeded";
      else if (complete) job.status = "partial";
      else if (remote.conclusion === "cancelled") job.status = "cancelled";
      else if (remote.conclusion !== "success") job.status = "failed";
      else job.status = "import_failed";
      job.finishedAt = now();
    }
    delete job.error;
    await this.jobs.persist(job);
  }
  async cancel(projectId, id) {
    return this.store.exclusive("@github-poll", async () => {
      const job = (await this.jobs.list(projectId)).find((j) => j.id === id && j.provider === "github");
      if (!job || !ongoing.has(job.status)) throw new StudioError("진행 중인 Actions 빌드가 없습니다.");
      job.cancelRequested = true;
      if (!job.github.dispatchAttemptedAt) { job.status = "cancelled"; job.finishedAt = now(); }
      else job.status = "cancelling";
      await this.jobs.persist(job); this.wake(); return job;
    });
  }
  async retryImport(projectId, id) {
    return this.store.exclusive("@github-poll", async () => {
      const job = (await this.jobs.list(projectId)).find((j) => j.id === id && j.provider === "github");
      if (!job?.github.runId) throw new StudioError("가져올 Actions 실행이 없습니다.");
      job.status = "downloading"; delete job.finishedAt;
      await this.jobs.persist(job); this.wake(); return job;
    });
  }
  async tick() {
    return this.store.exclusive("@github-poll", async () => {
      for (const project of await this.store.list()) for (const job of await this.jobs.list(project.id)) {
        if (this.closed || job.provider !== "github" || !ongoing.has(job.status)) continue;
        if (!this.api || this.profile.repository !== job.github.repository) {
          job.status = "awaiting_auth"; await this.jobs.persist(job); continue;
        }
        try {
          if (!job.github.headSha) await this.prepare(job);
          if (!job.github.dispatchAttemptedAt) await this.dispatch(job);
          await this.sync(job);
        } catch (e) {
          job.error = e.message;
          if (this.closed || !this.api || [401, 403].includes(e.githubStatus)) job.status = "awaiting_auth";
          else if (job.github.dispatchAttemptedAt) job.status = job.cancelRequested ? "cancelling" : "dispatch_unknown";
          else { job.status = "failed"; job.finishedAt = now(); }
          await this.jobs.persist(job);
        }
      }
    });
  }
  schedule() {
    if (this.closed) return;
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.wake(), this.pollInterval); this.timer.unref?.();
  }
  wake() {
    if (this.closed || this.pending) return;
    clearTimeout(this.timer);
    this.pending = this.tick().catch(() => {}).finally(() => { this.pending = null; this.schedule(); });
  }
  async close() { this.closed = true; clearTimeout(this.timer); this.api?.close(); await this.pending; }
}
