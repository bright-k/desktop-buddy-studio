import { pipeline } from "node:stream/promises";
import { createServer } from "node:http";
import { randomBytes, timingSafeEqual } from "node:crypto";
import * as fs from "node:fs/promises";
import { join, extname, dirname, basename } from "node:path";
import { fileURLToPath } from "node:url";
import { ProjectStore, StudioError, scopedFile, requireId } from "./store.mjs";
import { BuildQueue } from "./jobs.mjs";
import { run } from "./cli.mjs";
import { exportProject } from "./export.mjs";
import { ActionsBuilds } from "./github.mjs";
const defaultAppRoot = fileURLToPath(new URL("../apps/studio/dist", import.meta.url));
const mime = {
  ".html": "text/html; charset=utf-8",
  ".pdf": "application/pdf",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".wav": "audio/wav",
  ".mp3": "audio/mpeg",
  ".webp": "image/webp",
  ".bmp": "image/bmp",
  ".json": "application/json; charset=utf-8",
  ".txt": "text/plain; charset=utf-8",
  ".zip": "application/zip",
  ".dmg": "application/octet-stream",
};
async function body(req, limit = 2 * 1024 * 1024) {
  const chunks = [];
  let size = 0;
  for await (const chunk of req) {
    size += chunk.length;
    if (size > limit)
      throw new StudioError("요청 크기 제한을 초과했습니다.", 413);
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}
const safeEqual = (a, b) =>
  typeof a === "string" &&
  a.length === b.length &&
  timingSafeEqual(Buffer.from(a), Buffer.from(b));
export async function createStudio(options) {
  const token = options.token ?? randomBytes(32).toString("hex"),
    store = new ProjectStore(options),
    appRoot = options.appRoot ?? defaultAppRoot;
  await store.init({ migrate: false });
  const jobs = new BuildQueue(store, options);
  const actions = new ActionsBuilds(store, jobs, options);
  const lockPath = join(store.root, ".studio.lock");
  let workspaceLock;
  for (let attempt = 0; attempt < 2; attempt++) {
    try {
      workspaceLock = await fs.open(lockPath, "wx", 0o600);
      break;
    } catch (error) {
      if (error.code !== "EEXIST") throw error;
      const owner = JSON.parse(
        await fs.readFile(lockPath, "utf8").catch(() => "{}"),
      );
      let alive = false;
      if (Number.isInteger(owner.pid)) {
        try {
          process.kill(owner.pid, 0);
          alive = true;
        } catch (e) {
          if (e.code !== "ESRCH") alive = true;
        }
      }
      if (alive) {
        const error = new StudioError(
          "이 작업 폴더의 Studio가 이미 실행 중입니다.",
          409,
        );
        error.existingUrl = owner.url;
        error.existingVersion = owner.version;
        throw error;
      }
      await fs.rm(lockPath, { force: true });
    }
  }
  if (!workspaceLock) throw new StudioError("작업 폴더를 잠글 수 없습니다.");
  await workspaceLock.writeFile(JSON.stringify({ pid: process.pid }));
  try {
    await store.migratePackageDirectories();
  } catch (error) {
    await workspaceLock.close();
    await fs.rm(lockPath, { force: true });
    throw error;
  }
  let origin;
  const server = createServer(async (req, res) => {
    res.setHeader("Cache-Control", "no-store");
    res.setHeader("X-Content-Type-Options", "nosniff");
    res.setHeader("Referrer-Policy", "no-referrer");
    res.setHeader("Cross-Origin-Resource-Policy", "same-origin");
    res.setHeader(
      "Content-Security-Policy",
      "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob:; media-src 'self'; connect-src 'self'; object-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
    );
    const json = (value, status = 200) => {
      res.writeHead(status, {
        "Content-Type": "application/json; charset=utf-8",
      });
      res.end(JSON.stringify(value));
    };
    let apiRequest = false;
    const serve = async (path, download) => {
      const extension = extname(path).toLowerCase();
      const guide = extension === ".html" && apiRequest;
      if (guide || (extension === ".pdf" && apiRequest)) download ??= basename(path);
      const stat = await fs.stat(path);
      if (stat.size > 256 * 1024 * 1024 && !download)
        throw new StudioError("파일이 너무 큽니다.");
      res.writeHead(200, {
        "Content-Type": guide ? "application/octet-stream" : mime[extension] ?? "application/octet-stream",
        "Content-Length": stat.size,
        ...(download
          ? {
              "Content-Disposition": `attachment; filename*=UTF-8''${encodeURIComponent(download)}`,
            }
          : {}),
      });
      const handle = await fs.open(path);
      try {
        await pipeline(handle.createReadStream(), res);
      } finally {
        /* stream owns the handle */
      }
    };
    try {
      if (req.headers.host !== new URL(origin).host)
        throw new StudioError("Host rejected", 403);
      if (req.headers.origin && req.headers.origin !== origin)
        throw new StudioError("Origin rejected", 403);
      const url = new URL(req.url, origin),
        pathname = decodeURIComponent(url.pathname);
      apiRequest = pathname.startsWith("/api/");
      if (apiRequest) {
        const bearer = req.headers.authorization?.replace(/^Bearer /, "");
        const cookie = req.headers.cookie
          ?.split(";")
          .map((v) => v.trim())
          .find((v) => v.startsWith(`buddy_session_${new URL(origin).port}=`))
          ?.split("=")[1];
        if (!safeEqual(bearer, token) && !safeEqual(cookie, token))
          throw new StudioError("Studio 실행 주소로 다시 연결하세요.", 401);
        if (pathname === "/api/session" && req.method === "POST") {
          res.setHeader(
            "Set-Cookie",
            `buddy_session_${new URL(origin).port}=${token}; HttpOnly; SameSite=Strict; Path=/`,
          );
          return json({ ready: true });
        }
        if (pathname === "/api/config" && req.method === "GET")
          return json({
            dataRoot: store.root,
            target: jobs.target,
            development: jobs.development,
          });
        if (pathname === "/api/github" && req.method === "GET") return json(actions.status());
        if (pathname === "/api/github/connect" && req.method === "POST")
          return json(await actions.connect(JSON.parse(await body(req, 8192))));
        if (pathname === "/api/github/setup" && req.method === "POST") return json(await actions.setup());
        if (pathname === "/api/github/disconnect" && req.method === "POST") return json(await actions.disconnect());
        if (pathname === "/api/projects" && req.method === "GET")
          return json(await store.list());
        if (pathname === "/api/projects" && req.method === "POST")
          return json(await store.create(JSON.parse(await body(req))), 201);
        const match = pathname.match(/^\/api\/projects\/([^/]+)(?:\/(.*))?$/);
        if (!match) throw new StudioError("경로가 없습니다.", 404);
        const id = store.resolveId(match[1]),
          action = match[2] ?? "";
        if (!action && req.method === "GET") return json(await store.load(id));
        if (!action && req.method === "PUT")
          return json(await store.save(id, JSON.parse(await body(req))));
        if (action === "draft" && req.method === "PUT")
          return json(await store.saveDraft(id, JSON.parse(await body(req))));
        if (action === "draft" && req.method === "DELETE")
          return json(
            await store.discardDraft(
              id,
              JSON.parse(await body(req)).draftRevision,
            ),
          );
        if (action === "validate" && req.method === "POST")
          return json(await store.validate(id, JSON.parse(await body(req))));
        if (action === "clone" && req.method === "POST")
          return json(await store.clone(id), 201);
        if (action === "archive" && req.method === "POST")
          return json(
            await store.archive(id, JSON.parse(await body(req)).archived),
          );
        if (action === "revisions" && req.method === "GET")
          return json(await store.revisions(id));
        if (action === "restore" && req.method === "POST") {
          const b = JSON.parse(await body(req));
          return json(await store.restore(id, b.revision, b.expectedRevision, b.draftRevision));
        }
        if (action === "resources" && req.method === "POST")
          return json(
            await store.upload(
              id,
              await body(req, 16 * 1024 * 1024),
              url.searchParams.get("filename") ?? "",
              url.searchParams.get("purpose") === "installer",
            ),
            201,
          );
        if (action.startsWith("resources/") && req.method === "GET")
          return serve(
            await scopedFile(
              join(store.path(id), "resources"),
              action.slice(10),
            ),
          );
        const gifFrame = /^gif-frame\/([^/]+\.gif)\/([0-9]{1,3})$/.exec(action);
        if (gifFrame && req.method === "GET") {
          const source = await scopedFile(join(store.path(id), "resources"), gifFrame[1]);
          const stage = await fs.mkdtemp(join(store.root, ".staging", "gif-frame-"));
          try {
            const output = join(stage, "frame.png");
            await run(store.cli, ["media-info", source, "--frame", gifFrame[2], "--frame-output", output]);
            return await serve(output);
          } finally {
            await fs.rm(stage, { recursive: true, force: true });
          }
        }
        if (action.startsWith("source/") && req.method === "GET")
          return serve(
            await scopedFile(await store.currentPath(id), action.slice(7)),
          );
        if (action.startsWith("frames/") && req.method === "GET") {
          const [, revision, ...name] = action.split("/");
          requireId(revision);
          return serve(
            await scopedFile(
              join(store.path(id), "previews", revision),
              name.join("/"),
            ),
          );
        }
        if (action === "sequence" && req.method === "POST") {
          const b = JSON.parse(await body(req)),
            info = await store.inspectPath(await store.currentPath(id));
          const index = typeof b.id === "string"
            ? info.project.bindings.findIndex((binding) => binding.id === b.id)
            : b.index;
          const variant = b.variant ?? 0;
          if (!Number.isInteger(index) || !Number.isInteger(variant) || variant < 0)
            throw new StudioError("키워드 이벤트를 선택하세요.");
          const steps = variant === 0 ? info.sequences[index] : info.sequenceAlternatives?.[index]?.[variant - 1];
          if (!steps) throw new StudioError("키워드 입력을 선택하세요.");
          return json(steps);
        }
        if (action === "simulate" && req.method === "POST") {
          const b = JSON.parse(await body(req));
          if (
            !Array.isArray(b.events) ||
            b.events.length > 500 ||
            !Number.isInteger(b.untilMs) ||
            b.untilMs < 0 ||
            b.untilMs > 15000
          )
            throw new StudioError("미리보기는 최대 15초, 500개 이벤트입니다.");
          const current = await store.currentPath(id),
            temp = await fs.mkdtemp(
              join(store.root, ".staging", "simulation-"),
            );
          try {
            await fs.writeFile(
              join(temp, "events.json"),
              JSON.stringify(b.events),
            );
            const text = await run(store.cli, [
              "simulate",
              current,
              join(temp, "events.json"),
              "--until-ms",
              String(b.untilMs),
              "--timeline",
            ]);
            return json(text.trim().split("\n").map(JSON.parse));
          } finally {
            await fs.rm(temp, { recursive: true, force: true });
          }
        }
        if (action === "builds" && req.method === "GET")
          return json(await jobs.list(id));
        if (action.startsWith("export/") && req.method === "GET") {
          const archive = await exportProject(store, id, action.slice(7));
          return serve(archive.path, archive.name);
        }
        if (action === "builds" && req.method === "POST") {
          const b = JSON.parse(await body(req));
          if (b.provider === "github") return json(await actions.enqueue(id, b.revision, b), 202);
          return json(await jobs.enqueue(id, b.revision, b.development), 202);
        }
        if (
          /^builds\/[a-f0-9-]{36}\/cancel$/.test(action) &&
          req.method === "POST"
        ) {
          const jobId = action.split("/")[1], job = (await jobs.list(id)).find((j) => j.id === jobId);
          return json(await (job?.provider === "github" ? actions.cancel(id, jobId) : jobs.cancel(id, jobId)));
        }
        if (/^builds\/[a-f0-9-]{36}$/.test(action) && req.method === "DELETE") {
          const jobId = action.split("/")[1];
          const job = (await jobs.list(id)).find((j) => j.id === jobId);
          const removeFiles = url.searchParams.get("files") === "true";
          return json(await (job?.provider === "github" ? actions.removeHistory(id, jobId, removeFiles) : jobs.removeHistory(id, jobId, removeFiles)));
        }
        if (/^builds\/[a-f0-9-]{36}\/sync$/.test(action) && req.method === "POST")
          return json(await actions.retryImport(id, action.split("/")[1]));
        if (action.startsWith("download/") && req.method === "GET") {
          const [, jobId, ...parts] = action.split("/"),
            name = parts.join("/");
          const job = (await jobs.list(id)).find((j) => j.id === jobId),
            artifact = job?.downloads?.find((d) => d.name === name);
          if (!artifact)
            throw new StudioError("완료된 빌드 산출물이 없습니다.", 404);
          return serve(
            await scopedFile(join(store.path(id), "builds"), artifact.path),
            artifact.name,
          );
        }
        throw new StudioError("경로가 없습니다.", 404);
      }
      if (req.method !== "GET")
        throw new StudioError("허용되지 않는 요청입니다.", 405);
      return serve(
        await scopedFile(
          appRoot,
          pathname === "/" ? "index.html" : pathname.slice(1),
        ),
      );
    } catch (error) {
      if (!res.headersSent) json({ error: error.message }, error.status ?? 400);
      else res.destroy();
    }
  });
  try {
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(options.port ?? 4340, "127.0.0.1", resolve);
    });
  } catch (error) {
    await workspaceLock.close();
    await fs.rm(lockPath, { force: true });
    throw error;
  }
  origin = `http://127.0.0.1:${server.address().port}`;
  await jobs.init();
  await actions.init();
  await workspaceLock.truncate(0);
  await workspaceLock.write(
    JSON.stringify({ pid: process.pid, url: `${origin}/#${token}`, version: options.version }),
    0,
    "utf8",
  );
  return {
    server,
    store,
    jobs,
    actions,
    token,
    origin,
    url: `${origin}/#${token}`,
    async close() {
      await actions.close();
      await jobs.close();
      await new Promise((resolve) => server.close(resolve));
      await workspaceLock.close();
      await fs.rm(lockPath, { force: true });
    },
  };
}
