import { spawn } from "node:child_process";
import { toolchainEnvironment } from "../scripts/toolchain.mjs";
export function run(
  executable,
  args,
  { cwd, signal, onLog, maxBytes = 8 * 1024 * 1024 } = {},
) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, args, {
      cwd,
      env: toolchainEnvironment(),
      stdio: ["ignore", "pipe", "pipe"],
      windowsHide: true,
    });
    let stdout = "",
      stderr = "",
      overflow = false;
    const abort = () => {
      if (process.platform === "win32" && child.pid) {
        // Node cannot deliver a cooperative console SIGINT on Windows. Stop
        // the owned CLI tree so Cargo/NSIS do not survive a cancelled job.
        const killer = spawn(
          "taskkill",
          ["/PID", String(child.pid), "/T", "/F"],
          {
            windowsHide: true,
            stdio: "ignore",
          },
        );
        killer.once("error", reject);
      } else child.kill("SIGINT");
    };
    signal?.addEventListener("abort", abort, { once: true });
    if (signal?.aborted) abort();
    child.stdout.on("data", (data) => {
      stdout += data;
      if (stdout.length > maxBytes) {
        overflow = true;
        abort();
      }
    });
    child.stderr.on("data", (data) => {
      stderr = (stderr + data).slice(-65536);
      onLog?.(data.toString());
    });
    child.once("error", reject);
    child.once("close", (code) => {
      signal?.removeEventListener("abort", abort);
      if (code !== 0 || overflow) {
        const error = new Error(
          overflow
            ? "CLI output limit exceeded"
            : stderr.trim() || `CLI exited ${code}`,
        );
        error.stdout = stdout;
        reject(error);
      } else resolve(stdout);
    });
  });
}
export const jsonOutput = (text) => JSON.parse(text);
