const { spawnSync } = require("node:child_process");
const path = require("node:path");

const { loadWindowsBuildEnv } = require("./load-windows-build-env.cjs");

const rootDir = path.resolve(__dirname, "..");
const cliPath = path.join(rootDir, "node_modules", "@tauri-apps", "cli", "tauri.js");
const args = process.argv.slice(2);

const result = spawnSync(process.execPath, [cliPath, ...args], {
  cwd: rootDir,
  env: process.platform === "win32" ? loadWindowsBuildEnv(rootDir) : process.env,
  stdio: "inherit",
});

if (result.error) {
  throw result.error;
}

process.exit(result.status ?? 1);
