import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "..");
const outDir = mkdtempSync(join(tmpdir(), "codex-auth-gui-auto-switch-"));

try {
  writeFileSync(join(outDir, "package.json"), '{"type":"module"}');

  const tsc = join(
    repoRoot,
    "node_modules",
    ".bin",
    process.platform === "win32" ? "tsc.cmd" : "tsc",
  );
  const tscArgs = [
      "--target",
      "ES2022",
      "--module",
      "ES2022",
      "--moduleResolution",
      "Bundler",
      "--skipLibCheck",
      "--strict",
      "--noEmit",
      "false",
      "--outDir",
      outDir,
      join(repoRoot, "src", "auto-switch-policy.ts"),
  ];
  const command = process.platform === "win32" ? "cmd.exe" : tsc;
  const args = process.platform === "win32" ? ["/d", "/s", "/c", tsc, ...tscArgs] : tscArgs;

  execFileSync(
    command,
    args,
    { cwd: repoRoot, stdio: "inherit" },
  );

  const { refreshResultSupportsAutomation } = await import(
    pathToFileURL(join(outDir, "auto-switch-policy.js")).href
  );

  const completedStdout = [
    "[debug] usage refresh start: accounts=12",
    "[debug] usage refresh done: attempted=12 updated=4 failed=6 unchanged=2",
  ].join("\n");

  assert.equal(
    refreshResultSupportsAutomation({
      command: { success: true, stdout: "" },
      registry: { accounts: [] },
    }),
    true,
    "successful refresh should support automation",
  );

  assert.equal(
    refreshResultSupportsAutomation({
      command: { success: false, stdout: completedStdout },
      registry: { accounts: [{}] },
    }),
    true,
    "timed-out refresh with completed usage output should support automation",
  );

  assert.equal(
    refreshResultSupportsAutomation({
      command: { success: false, stdout: "[debug] usage refresh start: accounts=12" },
      registry: { accounts: [{}] },
    }),
    false,
    "unfinished refresh should not support automation",
  );

  assert.equal(
    refreshResultSupportsAutomation({
      command: { success: false, stdout: completedStdout },
      registry: { accounts: [] },
    }),
    false,
    "completed refresh without accounts should not support automation",
  );
} finally {
  rmSync(outDir, { recursive: true, force: true });
}
