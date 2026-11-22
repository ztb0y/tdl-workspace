import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const workspaceRoot = resolve(scriptDirectory, "..");
const buildScript = join(scriptDirectory, "build-client.ps1");
const installRoot = join(workspaceRoot, "bin", "client");
const launcher = join(installRoot, "tdl-web-launcher.exe");

function run(command, argumentsList, options = {}) {
  const result = spawnSync(command, argumentsList, { stdio: "inherit", ...options });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited with code ${result.status ?? "unknown"}`);
}

run("powershell.exe", ["-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", buildScript], {
  cwd: workspaceRoot,
});
run(launcher, [], {
  cwd: workspaceRoot,
  env: { ...process.env, TDL_WEB_INSTALL_ROOT: installRoot },
});
