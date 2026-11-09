import { test } from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs/promises";
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { createHash } from "node:crypto";
import { ProjectStore } from "./store.mjs";
import { BuildQueue } from "./jobs.mjs";
import { ActionsBuilds } from "./github.mjs";
import { GitHubAPI, repositoryName } from "./github-api.mjs";
import { artifactFiles, verifyArtifact } from "./github-artifacts.mjs";
import { run } from "./cli.mjs";
import { createStudio } from "./server.mjs";
import { tarExecutable } from "../scripts/archive-tools.mjs";

const cli = resolve("target/debug", process.platform === "win32" ? "buddy.exe" : "buddy");
const sha = (bytes) => createHash("sha256").update(bytes).digest("hex");
const blobHash = (bytes) => createHash("sha1").update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
const json = (value, status = 200) => new Response(JSON.stringify(value), { status });
class FakeGitHub {
  constructor() { this.blobs = new Map(); this.trees = new Map(); this.commits = new Map(); this.refs = new Map(); this.artifacts = []; this.dispatches = 0; this.completed = false; }
  fetch = async (address, options) => {
    assert.equal(options.headers.Authorization, "Bearer test-session-token");
    const url = new URL(address), path = url.pathname.replace("/repos/owner/repo", ""), body = options.body && JSON.parse(options.body);
    if (this.denied && path.startsWith("/actions")) return json({}, 403);
    if (url.pathname === "/user") return json({ login: "owner" });
    if (!path) return json({ default_branch: "main", private: true, permissions: { push: true } });
    if (path.startsWith("/contents/")) {
      if (options.method === "PUT") { this.workflow = body.content; return json({ commit: { sha: "setup" } }); }
      return this.workflow ? json({ encoding: "base64", content: this.workflow, sha: "workflow" }) : json({}, 404);
    }
    if (path === "/git/blobs") {
      const data = Buffer.from(body.content, "base64"), hash = blobHash(data); this.blobs.set(hash, data); return json({ sha: hash });
    }
    if (path === "/git/trees") {
      if (body.tree.some((node) => node.path === ".studio/request.json")) await this.beforeProjectTree?.();
      const entries = new Map(this.trees.get(body.base_tree) || []);
      for (const n of body.tree) {
        let hash = n.sha;
        if (n.content !== undefined) { const bytes = Buffer.from(n.content); hash = blobHash(bytes); this.blobs.set(hash, bytes); }
        entries.set(n.path, hash);
      }
      const hash = sha(JSON.stringify([...entries])); this.trees.set(hash, entries); return json({ sha: hash });
    }
    if (path.startsWith("/git/trees/")) {
      const entries = this.trees.get(path.split("/").at(-1));
      return json({ tree: [...entries].map(([path, sha]) => ({ path, sha, type: "blob", mode: "100644" })) });
    }
    if (path === "/git/commits") {
      const hash = sha(JSON.stringify(body)); this.commits.set(hash, { tree: { sha: body.tree } }); return json({ sha: hash });
    }
    if (path.startsWith("/git/commits/")) return json(this.commits.get(path.split("/").at(-1)));
    if (path === "/git/refs") {
      if (this.refs.has(body.ref)) return json({}, 422);
      this.refs.set(body.ref, body.sha); return json({ object: { sha: body.sha } });
    }
    if (path.startsWith("/git/ref/")) {
      const hash = this.refs.get("refs/" + path.slice("/git/ref/".length)); return hash ? json({ object: { sha: hash } }) : json({}, 404);
    }
    if (path.endsWith("/dispatches")) {
      this.dispatches++; this.ref = body.ref; this.head = this.refs.get("refs/heads/" + this.ref);
      if (this.timeout) throw new TypeError("network interruption after GitHub accepted dispatch");
      return json({ workflow_run_id: 42 });
    }
    const remote = { id: 42, head_sha: this.wrongHead ? "wrong" : this.head, head_branch: this.ref, event: "workflow_dispatch", run_attempt: 1, status: this.completed ? "completed" : "in_progress", conclusion: this.completed ? this.conclusion || "success" : null };
    if (path.endsWith("/workflows/studio-build.yml/runs")) return json({ workflow_runs: this.ref ? [remote] : [] });
    if (path === "/actions/runs/42") return json(remote);
    if (path === "/actions/runs/42/jobs") return json({ jobs: ["macos-arm64", "windows-x64"].map((name) => ({ name, status: remote.status, conclusion: remote.conclusion, steps: [{ name: "Build independent app", status: remote.status, conclusion: remote.conclusion }] })) });
    if (path === "/actions/runs/42/artifacts") return json({ artifacts: this.artifacts });
    if (/\/actions\/artifacts\/\d+\/zip$/.test(path)) return new Response(this.zip.get(Number(path.split("/").at(-2))));
    if (path === "/actions/runs/42/cancel") { this.cancelled = true; return new Response(null, { status: 202 }); }
    throw new Error(`Unexpected fake GitHub path: ${path}`);
  };
}
async function fixture(t) {
  const root = await fs.mkdtemp(join(tmpdir(), ".buddy-github-test-"));
  const store = new ProjectStore({ dataRoot: join(root, "data"), cli }); await store.init();
  const doc = await store.create({ name: "Remote Buddy", packageId: "com.example.remote" });
  const pack = join(root, "pack"); await run(cli, ["template-pack", "--source", resolve("."), "--output", pack]);
  const jobs = new BuildQueue(store), fake = new FakeGitHub();
  const options = { templatePack: pack, githubFetch: fake.fetch, credentialProvider: async () => ({ token: "test-session-token", source: "git-credential" }), pollInterval: 3600000 };
  const remote = new ActionsBuilds(store, jobs, options); remote.wake = () => {};
  await remote.init();
  t.after(async () => { await remote.close(); await fs.rm(root, { recursive: true, force: true }); });
  return { root, store, doc, jobs, fake, remote, options };
}
async function completeArtifact(root, fake, job, target, id) {
  const dir = join(root, target), bundle = target === "windows-x64" ? "b".repeat(140) : "bundle";
  await fs.mkdir(join(dir, bundle), { recursive: true });
  const installer = target === "windows-x64" ? "buddy-setup.exe" : "Buddy.dmg", binary = Buffer.from("fixture installer");
  await fs.writeFile(join(dir, bundle, installer), binary);
  const guide = "Installation Guide.pdf", pdf = Buffer.from("fixture PDF guide");
  await fs.writeFile(join(dir, bundle, guide), pdf);
  // macOS tar may preserve AppleDouble metadata beside the actual installer.
  await fs.writeFile(join(dir, bundle, "._" + installer), "metadata");
  const report = { projectHash: job.contentHash, identifier: job.projectId, templateHash: job.github.engineHash, target, development: true, staticInspection: "passed",
    artifacts: [{ path: installer, bytes: binary.length, sha256: sha(binary) }, { path: guide, bytes: pdf.length, sha256: sha(pdf) }] };
  await fs.writeFile(join(dir, "source.json"), JSON.stringify({ contentHash: job.contentHash, identifier: job.projectId }));
  await fs.writeFile(join(dir, bundle, "build-report.json"), JSON.stringify(report));
  const name = `project-${target}.tar.gz`;
  await run(tarExecutable, ["-czf", join(dir, name), "-C", dir, "source.json", bundle]);
  const bytes = await fs.readFile(join(dir, name)), receipt = `receipt-${target}.json`;
  await fs.writeFile(join(dir, receipt), JSON.stringify({ jobId: job.id, projectHash: job.contentHash, identifier: job.projectId, target, templateHash: job.github.engineHash, engineHash: job.github.engineHash, bytes: bytes.length, sha256: sha(bytes) }));
  const zipPath = join(dir, "artifact.zip");
  await run(tarExecutable, ["-a", "-cf", zipPath, "-C", dir, name, receipt]);
  const zip = await fs.readFile(zipPath); fake.zip ||= new Map(); fake.zip.set(id, zip);
  const artifact = { id, name: `studio-${job.id}-${target}-a1`, digest: `sha256:${sha(zip)}`, expired: false };
  fake.artifacts.push(artifact); return { zip, artifact };
}

test("GitHub connection, workflow setup, frozen upload, double-click dedup, cancellation and restart preserve credentials and run identity", async (t) => {
  const { root, store, doc, jobs, fake, remote, options } = await fixture(t);
  await remote.connect({ repository: "owner/repo" }); assert.equal(remote.status().ready, false);
  await remote.setup(); assert.equal(remote.status().ready, true);
  await assert.rejects(remote.enqueue(doc.meta.id, doc.meta.revision, { acknowledgeUpload: true, repository: "owner/another" }), (e) => e.status === 409);
  const [a, b] = await Promise.all([remote.enqueue(doc.meta.id, doc.meta.revision, { acknowledgeUpload: true }), remote.enqueue(doc.meta.id, doc.meta.revision, { acknowledgeUpload: true })]);
  assert.equal(a.id, b.id);
  await remote.tick(); let job = (await jobs.list(doc.meta.id))[0];
  assert.equal(fake.dispatches, 1); assert.equal(job.github.runId, 42); assert.equal(job.status, "running");
  const tree = fake.trees.get(fake.commits.get(job.github.headSha).tree.sha);
  assert.ok(tree.has(".studio/project/buddy.project.json")); assert.ok(tree.has(".gitattributes")); assert.ok(![...tree.keys()].some((p) => p.startsWith(".data/") || p.includes("draft.json")));
  await jobs.init(); assert.equal((await jobs.list(doc.meta.id))[0].status, "running");
  assert.doesNotMatch(await fs.readFile(join(store.root, "github.json"), "utf8"), /test-session-token/);
  await remote.close();
  const resumed = new ActionsBuilds(store, jobs, options); resumed.wake = () => {}; await resumed.init(); t.after(() => resumed.close());
  await resumed.tick(); assert.equal(fake.dispatches, 1);
  await resumed.cancel(doc.meta.id, job.id); await resumed.tick(); assert.equal(fake.cancelled, true);
  assert.equal((await jobs.list(doc.meta.id))[0].github.cancelSent, true);
  await resumed.disconnect(); await resumed.tick(); assert.equal((await jobs.list(doc.meta.id))[0].status, "awaiting_auth");
});
test("cancelling while a project is uploading responds before the upload finishes and does not dispatch", async (t) => {
  const { doc, jobs, fake, remote } = await fixture(t);
  await remote.connect({ repository: "owner/repo" }); await remote.setup();
  const entered = Promise.withResolvers(), release = Promise.withResolvers();
  fake.beforeProjectTree = async () => { entered.resolve(); await release.promise; };
  const job = await remote.enqueue(doc.meta.id, doc.meta.revision, { acknowledgeUpload: true });
  const tick = remote.tick();
  await entered.promise;
  try {
    const deadline = AbortSignal.timeout(2000);
    const cancelled = await Promise.race([remote.cancel(doc.meta.id, job.id), new Promise((_, reject) => deadline.addEventListener("abort", () => reject(new Error("cancel waited for upload")), { once: true }))]);
    assert.equal(cancelled.status, "cancelled");
  } finally { release.resolve(); await tick; }
  assert.equal(fake.dispatches, 0);
  assert.equal(fake.refs.has(`refs/heads/studio-build/${job.id}`), false);
  assert.equal((await jobs.list(doc.meta.id))[0].status, "cancelled");
});
test("dispatch timeout recovers without duplicate runs; completed platform artifacts are verified and imported, partial success is preserved", async (t) => {
  const { root, store, doc, jobs, fake, remote } = await fixture(t);
  await remote.connect({ repository: "owner/repo" }); await remote.setup();
  await remote.enqueue(doc.meta.id, doc.meta.revision, { acknowledgeUpload: true }); fake.timeout = true;
  await remote.tick(); assert.equal((await jobs.list(doc.meta.id))[0].status, "dispatch_unknown");
  await remote.tick(); let job = (await jobs.list(doc.meta.id))[0]; assert.equal(job.status, "running"); assert.equal(fake.dispatches, 1);
  const first = await completeArtifact(root, fake, job, "macos-arm64", 1);
  assert.throws(() => verifyArtifact(first.zip, { ...first.artifact, digest: "sha256:" + "0".repeat(64) }, job, "macos-arm64"));
  const actualDigest = first.artifact.digest; first.artifact.digest = "sha256:" + "0".repeat(64);
  fake.completed = true; fake.conclusion = "failure"; await remote.tick(); job = (await jobs.list(doc.meta.id))[0];
  assert.equal(job.status, "import_failed"); assert.equal(job.downloads, undefined);
  first.artifact.digest = actualDigest; await remote.retryImport(doc.meta.id, job.id); await remote.tick(); job = (await jobs.list(doc.meta.id))[0];
  assert.equal(job.status, "partial"); assert.equal(job.platforms["macos-arm64"].imported, true);
  await completeArtifact(root, fake, job, "windows-x64", 2);
  fake.conclusion = "success"; await remote.retryImport(doc.meta.id, job.id); await remote.tick(); job = (await jobs.list(doc.meta.id))[0];
  assert.equal(job.status, "succeeded"); assert.equal(job.downloads.length, 9);
  const guide = job.downloads.find(d => d.name === "Installation Guide.pdf");
  assert.equal(await fs.readFile(join(store.path(doc.meta.id), "builds", guide.path), "utf8"), "fixture PDF guide"); assert.equal(fake.dispatches, 1);
  const zip = Buffer.from(first.zip); zip[50] ^= 1;
  assert.throws(() => artifactFiles(zip, ["project-macos-arm64.tar.gz", "receipt-macos-arm64.json"]));
  assert.throws(() => artifactFiles(first.zip, ["../escape", "receipt-macos-arm64.json"]));
});
test("artifact redirects omit credentials, unknown hosts and invalid repository names are rejected", async () => {
  assert.throws(() => repositoryName("owner/repo?token=secret"));
  assert.throws(() => repositoryName("owner/.."));
  let calls = 0;
  const api = new GitHubAPI("never-forward-this", { fetcher: async (url, options) => {
    calls++;
    if (calls === 1) { assert.ok(options.headers.Authorization); return new Response(null, { status: 302, headers: { location: "https://test.blob.core.windows.net/signed" } }); }
    assert.equal(options.headers, undefined); return new Response("download");
  } });
  assert.equal((await api.request("/artifact", { binary: true })).toString(), "download");
  const bad = new GitHubAPI("secret", { fetcher: async () => new Response(null, { status: 302, headers: { location: "https://attacker.example/signed" } }) });
  await assert.rejects(bad.request("/artifact", { binary: true }), /GitHub/);
});
test("expired authorization pauses remote work and reconnect resumes it; a mismatched run is rejected without downloads", async (t) => {
  const { doc, jobs, fake, remote } = await fixture(t);
  await remote.connect({ repository: "owner/repo" }); await remote.setup();
  await remote.enqueue(doc.meta.id, doc.meta.revision, { acknowledgeUpload: true }); await remote.tick();
  fake.denied = true; await remote.tick();
  assert.equal(remote.status().connected, false);
  assert.equal((await jobs.list(doc.meta.id))[0].status, "awaiting_auth");
  fake.denied = false; await remote.connect({ repository: "owner/repo" }); await remote.tick();
  assert.equal((await jobs.list(doc.meta.id))[0].status, "running"); assert.equal(fake.dispatches, 1);
  fake.wrongHead = true; await remote.tick(); const job = (await jobs.list(doc.meta.id))[0];
  assert.equal(job.status, "failed"); assert.match(job.error, /일치하지/); assert.equal(job.downloads, undefined);
});
test("GitHub HTTP endpoints require the Studio session and origin, and never return or persist a supplied credential", async (t) => {
  const { store, options } = await fixture(t);
  const server = await createStudio({ ...options, dataRoot: store.root, cli, port: 0 });
  t.after(() => server.close());
  assert.equal((await fetch(server.origin + "/api/github")).status, 401);
  const request = (path, extra = {}) => fetch(server.origin + path, { method: "POST", body: JSON.stringify({ repository: "owner/repo", token: "test-session-token" }),
    ...extra, headers: { Authorization: `Bearer ${server.token}`, "Content-Type": "application/json", ...extra.headers } });
  assert.equal((await request("/api/github/connect", { headers: { Origin: "https://attacker.example" } })).status, 403);
  const response = await request("/api/github/connect"); assert.equal(response.status, 200);
  assert.doesNotMatch(await response.text(), /test-session-token/);
  assert.doesNotMatch(await fs.readFile(join(store.root, "github.json"), "utf8"), /test-session-token/);
});
