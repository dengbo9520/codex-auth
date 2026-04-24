const { spawnSync } = require("node:child_process");
const path = require("node:path");

const { loadWindowsBuildEnv } = require("./load-windows-build-env.cjs");

const rootDir = path.resolve(__dirname, "..");
const [command, ...args] = process.argv.slice(2);

if (!command) {
  console.error("Usage: node scripts/run-with-vcvars.cjs <command> [args...]");
  process.exit(1);
}

const result = spawnSync(command, args, {
  cwd: rootDir,
  env: process.platform === "win32" ? loadWindowsBuildEnv(rootDir) : process.env,
  stdio: "inherit",
  shell: false,
});

if (result.error) {
  throw result.error;
}

process.exit(result.status ?? 1);
