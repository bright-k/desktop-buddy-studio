import { join } from "node:path";

// Git Bash's GNU tar treats D:\... as a remote host. Use Windows' bundled
// bsdtar explicitly, including when Studio/CI is launched from Git Bash.
export const tarExecutable = process.platform === "win32"
  ? join(process.env.SystemRoot || "C:\\Windows", "System32", "tar.exe")
  : "tar";
