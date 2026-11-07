import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile, mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import ts from "typescript";
test("project, tab, animation and event survive a URL roundtrip without dropping unrelated query state", async () => {
  const temp = await mkdtemp(join(tmpdir(), "buddy-route-"));
  try {
    const source = await readFile(
      new URL("../apps/studio/src/route.ts", import.meta.url),
      "utf8",
    );
    await writeFile(
      join(temp, "route.mjs"),
      ts.transpileModule(source, {
        compilerOptions: {
          target: ts.ScriptTarget.ES2022,
          module: ts.ModuleKind.ESNext,
        },
      }).outputText,
    );
    const { readRoute, writeRoute } = await import(
      pathToFileURL(join(temp, "route.mjs"))
    );
    const state = {
      projectId: "com.example.testbuddy",
      tab: "animations",
      clip: "my-gif",
      bindingId: "secret-cat",
    };
    const url = writeRoute(new URL("http://127.0.0.1:4340/?extra=keep"), state);
    assert.deepEqual(readRoute(url), state);
    assert.equal(
      readRoute(
        new URL(
          "http://localhost/?project=11111111-2222-4333-8444-555555555555",
        ),
      ).projectId,
      "11111111-2222-4333-8444-555555555555",
    );
    assert.equal(url.searchParams.get("extra"), "keep");
    assert.deepEqual(
      readRoute(
        new URL(
          "http://localhost/?project=../invalid&tab=other&clip=%3Cscript%3E",
        ),
      ),
      { projectId: null, tab: "overview", clip: "idle", bindingId: null },
    );
  } finally {
    await rm(temp, { recursive: true, force: true });
  }
});
