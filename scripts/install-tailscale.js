const { spawnSync } = require("child_process");

function commandExists(command) {
  const checker = process.platform === "win32" ? "where" : "command";
  const args = process.platform === "win32" ? [command] : ["-v", command];
  return spawnSync(checker, args, { stdio: "ignore", shell: process.platform !== "win32" }).status === 0;
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { stdio: "inherit", shell: false, ...options });
  return result.status === 0;
}

function installMac() {
  if (!commandExists("brew")) {
    console.error("Homebrew was not found. Install Homebrew first, then run this command again.");
    return false;
  }
  const installed = run("brew", ["install", "--cask", "tailscale"]);
  run("open", ["-a", "Tailscale"]);
  return installed;
}

function installWindows() {
  if (!commandExists("winget")) {
    console.error("winget was not found. Install App Installer from Microsoft Store, then run this command again.");
    return false;
  }
  return run("winget", [
    "install",
    "--id",
    "Tailscale.Tailscale",
    "--exact",
    "--source",
    "winget",
    "--accept-package-agreements",
    "--accept-source-agreements"
  ]);
}

function installLinux() {
  console.error("Automatic Tailscale install is currently supported for macOS and Windows only.");
  return false;
}

const ok = process.platform === "darwin"
  ? installMac()
  : process.platform === "win32"
    ? installWindows()
    : installLinux();

process.exit(ok ? 0 : 1);
