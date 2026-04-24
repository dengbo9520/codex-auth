const { spawnSync } = require("node:child_process");
const path = require("node:path");

function loadWindowsBuildEnv(rootDir, baseEnv = process.env) {
  const helperPath = path.join(rootDir, "scripts", "with-vcvars.cmd");
  const envDump = spawnSync(helperPath, ["--print-env"], {
    cwd: rootDir,
    env: baseEnv,
    encoding: "utf8",
    maxBuffer: 1024 * 1024 * 8,
    shell: true,
  });

  if (envDump.status !== 0) {
    process.stderr.write(
      envDump.stderr || envDump.stdout || "Failed to load Visual C++ build environment.\n",
    );
    process.exit(envDump.status ?? 1);
  }

  const loadedEnv = { ...baseEnv };

  for (const line of envDump.stdout.split(/\r?\n/)) {
    const dividerIndex = line.indexOf("=");
    if (dividerIndex <= 0) {
      continue;
    }

    const key = line.slice(0, dividerIndex);
    const value = line.slice(dividerIndex + 1);
    loadedEnv[key] = value;
  }

  const mergedPath = loadedEnv.Path || loadedEnv.PATH || baseEnv.Path || baseEnv.PATH || "";
  loadedEnv.Path = mergedPath;
  loadedEnv.PATH = mergedPath;

  return loadedEnv;
}

module.exports = { loadWindowsBuildEnv };
