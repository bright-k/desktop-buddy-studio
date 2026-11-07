import { spawn } from "node:child_process";
import { StudioError } from "./store.mjs";

export function repositoryName(value) {
  if (typeof value !== "string" || !/^[a-zA-Z0-9][a-zA-Z0-9-]{0,38}\/[a-zA-Z0-9_.-]{1,100}$/.test(value) || value.endsWith("/.") || value.endsWith("/.."))
    throw new StudioError("GitHub 저장소를 owner/repository 형식으로 입력하세요.");
  return value;
}
export async function githubCredential() {
  if (process.env.GH_TOKEN || process.env.GITHUB_TOKEN)
    return { token: process.env.GH_TOKEN || process.env.GITHUB_TOKEN, source: "environment" };
  return new Promise((resolve) => {
    const child = spawn("git", ["credential", "fill"], {
      env: { ...process.env, GIT_TERMINAL_PROMPT: "0" },
      stdio: ["pipe", "pipe", "ignore"], windowsHide: true,
    });
    let output = "";
    const timer = setTimeout(() => child.kill(), 5000);
    child.stdout.on("data", (chunk) => { output += chunk; if (output.length > 16384) child.kill(); });
    child.on("error", () => { clearTimeout(timer); resolve(null); });
    child.on("close", (code) => {
      clearTimeout(timer);
      const token = output.split("\n").find((line) => line.startsWith("password="))?.slice(9).trim();
      output = "";
      resolve(code === 0 && token ? { token, source: "git-credential" } : null);
    });
    child.stdin.on("error", () => {});
    child.stdin.end("protocol=https\nhost=github.com\n\n");
  });
}
export class GitHubAPI {
  constructor(token, { fetcher = fetch } = {}) { this.token = token; this.fetcher = fetcher; this.controller = new AbortController(); }
  close() { this.controller.abort(); this.token = ""; }
  async request(path, { method = "GET", body, binary = false, maxBytes = 8 * 1024 * 1024 } = {}) {
    if (!path.startsWith("/") || path.startsWith("//")) throw new Error("Invalid GitHub API path");
    const response = await this.fetcher("https://api.github.com" + path, {
      method, redirect: "manual", signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(60000)]),
      headers: { Accept: "application/vnd.github+json", Authorization: `Bearer ${this.token}`,
        "X-GitHub-Api-Version": "2022-11-28", "User-Agent": "Desktop-Buddy-Studio",
        ...(body ? { "Content-Type": "application/json" } : {}) },
      ...(body ? { body: JSON.stringify(body) } : {}),
    });
    if (binary && response.status === 302) {
      const destination = new URL(response.headers.get("location"));
      if (destination.protocol !== "https:" || destination.username || destination.password ||
          !["github.com", ".githubusercontent.com", ".blob.core.windows.net", ".actions.githubusercontent.com"].some((host) => host.startsWith(".") ? destination.hostname.endsWith(host) : destination.hostname === host))
        throw new StudioError("GitHub 다운로드 주소를 확인할 수 없습니다.");
      // A signed artifact URL must never receive the repository credential.
      const download = await this.fetcher(destination.href, { redirect: "error", signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(120000)]) });
      if (!download.ok) throw new StudioError("GitHub 결과물 다운로드에 실패했습니다. 다시 동기화하세요.");
      return readBounded(download, maxBytes);
    }
    if (!response.ok) {
      const messages = { 401: "GitHub 인증이 만료됐습니다. 다시 연결하세요.", 403: "GitHub 권한·Actions 정책 또는 API 사용 한도를 확인하세요.",
        404: "GitHub 저장소·워크플로가 없거나 접근 권한이 없습니다.", 409: "GitHub 브랜치가 변경됐습니다. 다시 확인하세요.",
        422: "GitHub 요청이 거부됐습니다. 워크플로 등록·브랜치 보호·토큰의 Workflows 쓰기 권한을 확인하세요." };
      const error = new StudioError(messages[response.status] || `GitHub 요청 실패 (${response.status}).`, response.status);
      error.githubStatus = response.status;
      throw error;
    }
    if (response.status === 204) return null;
    if (binary) return readBounded(response, maxBytes);
    return JSON.parse((await readBounded(response, maxBytes)).toString("utf8"));
  }
}
async function readBounded(response, limit) {
  if (Number(response.headers.get("content-length")) > limit) throw new StudioError("GitHub 응답 크기 제한을 초과했습니다.");
  const chunks = []; let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > limit) throw new StudioError("GitHub 응답 크기 제한을 초과했습니다.");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}
