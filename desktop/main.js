const path = require("path");
const fs = require("fs");
const os = require("os");
const { spawn, spawnSync } = require("child_process");
const { app, BrowserWindow, desktopCapturer, dialog, ipcMain, screen, session, shell } = require("electron");
const { startServer } = require("../server");

let mainWindow = null;
let serverHandle = null;
let nativeInputWorker = null;
const nativeProcesses = {
  client: null,
  host: null
};
const nativeLogBuffers = {
  client: "",
  host: ""
};
const nativeLogs = {
  client: [],
  host: []
};

const gotLock = app.requestSingleInstanceLock();

if (!gotLock) {
  app.quit();
}

app.setName("Sanser");
app.commandLine.appendSwitch("enable-features", "WebRTCPipeWireCapturer");

app.on("second-instance", () => {
  if (!mainWindow) return;
  if (mainWindow.isMinimized()) mainWindow.restore();
  mainWindow.focus();
});

app.whenReady().then(boot).catch((error) => {
  dialog.showErrorBox("GameRemote failed to start", error.message);
  app.quit();
});

app.on("activate", () => {
  if (BrowserWindow.getAllWindows().length === 0 && serverHandle) {
    createWindow(`http://127.0.0.1:${serverHandle.port}`);
  }
});

app.on("before-quit", () => {
  if (serverHandle?.server) {
    serverHandle.server.close();
  }
  if (nativeInputWorker) {
    nativeInputWorker.kill();
  }
  stopNativeProcess("client");
  stopNativeProcess("host");
});

async function boot() {
  process.env.GAME_REMOTE_DATA_DIR = path.join(app.getPath("userData"), "data");
  await ensureTailscaleReady();
  installScreenCaptureHandler();
  installInputHandler();
  installNativeHandlers();
  serverHandle = await startEmbeddedServer();
  createWindow(`http://127.0.0.1:${serverHandle.port}`);
}

async function ensureTailscaleReady() {
  if (process.env.NETWORK_MODE !== "tailscale") return;
  if (isTailscaleInstalled()) {
    await startTailscale();
    await ensureTailscaleLoggedIn();
    return;
  }

  const choice = await dialog.showMessageBox({
    type: "question",
    buttons: ["Cài Tailscale", "Để sau"],
    defaultId: 0,
    cancelId: 1,
    title: "Cần Tailscale",
    message: "Sanser đang dùng Tailscale để kết nối khác mạng.",
    detail: "Máy này chưa có Tailscale. Sanser có thể tự cài theo hệ điều hành rồi bạn đăng nhập cùng tài khoản Tailscale trên cả hai máy."
  });
  if (choice.response !== 0) return;

  try {
    await installTailscale();
    await startTailscale();
    await ensureTailscaleLoggedIn();
  } catch (error) {
    dialog.showErrorBox("Không cài được Tailscale", error.message);
  }
}

function isTailscaleInstalled() {
  if (getTailscaleCommand()) return true;
  if (process.platform === "darwin") {
    return fs.existsSync("/Applications/Tailscale.app");
  }
  if (process.platform === "win32") {
    return [
      path.join(process.env.ProgramFiles || "C:\\Program Files", "Tailscale", "tailscale.exe"),
      path.join(process.env["ProgramFiles(x86)"] || "C:\\Program Files (x86)", "Tailscale", "tailscale.exe"),
      path.join(process.env.LOCALAPPDATA || "", "Tailscale", "tailscale.exe")
    ].some((candidate) => candidate && fs.existsSync(candidate));
  }
  return false;
}

function getTailscaleCommand() {
  const candidates = process.platform === "win32"
    ? [
        "tailscale.exe",
        path.join(process.env.ProgramFiles || "C:\\Program Files", "Tailscale", "tailscale.exe"),
        path.join(process.env["ProgramFiles(x86)"] || "C:\\Program Files (x86)", "Tailscale", "tailscale.exe"),
        path.join(process.env.LOCALAPPDATA || "", "Tailscale", "tailscale.exe")
      ]
    : [
        "tailscale",
        "/usr/local/bin/tailscale",
        "/opt/homebrew/bin/tailscale"
      ];

  for (const candidate of candidates) {
    if (!candidate) continue;
    if (candidate.includes(path.sep) && fs.existsSync(candidate)) return candidate;
    if (!candidate.includes(path.sep) && commandExists(candidate)) return candidate;
  }
  return "";
}

function commandExists(command) {
  const checker = process.platform === "win32" ? "where" : "command";
  const args = process.platform === "win32" ? [command] : ["-v", command];
  return spawnSync(checker, args, { stdio: "ignore", shell: process.platform !== "win32" }).status === 0;
}

function installTailscale() {
  if (process.platform === "darwin") {
    if (!commandExists("brew")) {
      throw new Error("Máy Mac chưa có Homebrew nên Sanser không thể tự cài Tailscale. Hãy cài Homebrew hoặc cài Tailscale một lần thủ công.");
    }
    return runInstaller("brew", ["install", "--cask", "tailscale"]);
  }

  if (process.platform === "win32") {
    if (!commandExists("winget")) {
      throw new Error("Windows chưa có winget/App Installer nên Sanser không thể tự cài Tailscale. Hãy cài App Installer từ Microsoft Store rồi mở lại Sanser.");
    }
    return runInstaller("winget", [
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

  throw new Error("Tự cài Tailscale hiện chỉ hỗ trợ macOS và Windows.");
}

function runInstaller(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      stdio: "ignore",
      windowsHide: false
    });
    child.on("error", reject);
    child.on("close", (code) => {
      if (code === 0) resolve();
      else reject(new Error(`${command} exited with code ${code}.`));
    });
  });
}

function openTailscale() {
  if (process.platform === "darwin" && fs.existsSync("/Applications/Tailscale.app")) {
    spawn("open", ["-a", "Tailscale"], { stdio: "ignore", detached: true }).unref();
    return;
  }
  if (process.platform === "win32") {
    spawn("cmd", ["/c", "start", "", "tailscale:"], { stdio: "ignore", detached: true }).unref();
  }
}

async function startTailscale() {
  openTailscale();
  await delay(2000);
}

async function ensureTailscaleLoggedIn() {
  const tailscale = getTailscaleCommand();
  if (!tailscale) return;
  const status = spawnSync(tailscale, ["status"], {
    encoding: "utf8",
    timeout: 5000,
    windowsHide: true
  });
  const output = `${status.stdout || ""}\n${status.stderr || ""}`;
  if (status.status === 0 && !/Logged out|NeedsLogin|stopped|failed/i.test(output)) return;

  const choice = await dialog.showMessageBox({
    type: "question",
    buttons: ["Đăng nhập Tailscale", "Để sau"],
    defaultId: 0,
    cancelId: 1,
    title: "Đăng nhập Tailscale",
    message: "Sanser cần Tailscale đã đăng nhập để kết nối khác mạng.",
    detail: "Sanser sẽ mở luồng đăng nhập Tailscale. Hãy dùng cùng một tài khoản Tailscale trên cả Mac và Windows."
  });
  if (choice.response !== 0) return;

  spawn(tailscale, ["up"], {
    stdio: "ignore",
    detached: true,
    windowsHide: false
  }).unref();
}

function delay(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function startEmbeddedServer() {
  try {
    return await startServer({ host: "0.0.0.0", port: Number(process.env.PORT || 5174) });
  } catch (error) {
    if (error.code !== "EADDRINUSE") throw error;
    return startServer({ host: "127.0.0.1", port: 0 });
  }
}

function createWindow(url) {
  mainWindow = new BrowserWindow({
    width: 1360,
    height: 860,
    minWidth: 1040,
    minHeight: 700,
    backgroundColor: "#101110",
    title: "Sanser",
    autoHideMenuBar: true,
    webPreferences: {
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
      preload: path.join(__dirname, "preload.js")
    }
  });

  mainWindow.webContents.setWindowOpenHandler(({ url: externalUrl }) => {
    shell.openExternal(externalUrl);
    return { action: "deny" };
  });

  mainWindow.loadURL(url);
  mainWindow.on("closed", () => {
    mainWindow = null;
  });
}

function installInputHandler() {
  ipcMain.on("sanser:host-input", (_event, payload = {}) => {
    if (!mainWindow || mainWindow.isDestroyed()) return;
    sendHostInput(payload);
  });
}

function installNativeHandlers() {
  ipcMain.handle("sanser:native-status", () => nativeStatus());
  ipcMain.handle("sanser:native-network-info", () => nativeNetworkInfo());
  ipcMain.handle("sanser:native-start-client", (_event, options = {}) => startNativeClient(options));
  ipcMain.handle("sanser:native-stop-client", () => stopNativeProcess("client"));
  ipcMain.handle("sanser:native-start-host", (_event, options = {}) => startNativeHost(options));
  ipcMain.handle("sanser:native-stop-host", () => stopNativeProcess("host"));
}

function startNativeClient(options = {}) {
  if (process.platform !== "darwin") {
    throw new Error("Native SNV client hiện chỉ hỗ trợ macOS.");
  }

  stopNativeProcess("client");
  const executable = resolveNativeClientPath();
  if (!fs.existsSync(executable)) {
    throw new Error(`Không tìm thấy sanser-native-client tại ${executable}. Hãy build bằng npm run native:client-mac:build.`);
  }

  const port = clampInt(options.port, 1, 65535, 7777);
  const maxPackets = clampInt(options.maxPackets, 0, Number.MAX_SAFE_INTEGER, 0);
  const args = ["--listen-render-snv", String(port)];
  if (maxPackets > 0) args.push("--max-packets", String(maxPackets));
  if (options.logInput !== false) args.push("--log-input");
  if (options.fullscreen !== false) args.push("--fullscreen");
  if (options.hideCursor !== false) args.push("--hide-cursor");

  nativeProcesses.client = spawnNativeProcess("client", executable, args);
  return nativeStatus();
}

function startNativeHost(options = {}) {
  if (process.platform !== "win32") {
    throw new Error("Native SNV host hiện chỉ hỗ trợ Windows.");
  }

  const endpoint = String(options.endpoint || "").trim();
  if (!/^\[?[A-Za-z0-9:._-]+\]?:\d+$/.test(endpoint)) {
    throw new Error("Native host cần endpoint dạng HOST:PORT.");
  }

  stopNativeProcess("host");
  const executable = resolveNativeHostPath();
  if (!fs.existsSync(executable)) {
    throw new Error(`Không tìm thấy sanser-native-host.exe tại ${executable}. Hãy build bằng npm run native:host-win:build.`);
  }

  const fps = clampInt(options.fps, 30, 120, 60);
  const bitrateMbps = clampInt(options.bitrateMbps, 4, 120, 28);
  const args = [
    "--encode-pipe",
    "h264",
    "--fps",
    String(fps),
    "--interval-ms",
    "0",
    "--bitrate",
    String(bitrateMbps * 1000000),
    "--tcp-connect",
    endpoint
  ];
  if (options.softwareEncoder) args.push("--software-encoder");

  nativeProcesses.host = spawnNativeProcess("host", executable, args);
  return nativeStatus();
}

function spawnNativeProcess(role, executable, args) {
  nativeLogs[role] = [];
  nativeLogBuffers[role] = "";
  appendNativeLog(role, `$ ${path.basename(executable)} ${args.join(" ")}`);

  const child = spawn(executable, args, {
    cwd: path.dirname(executable),
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: false
  });

  child.stdout.on("data", (chunk) => consumeNativeLog(role, chunk));
  child.stderr.on("data", (chunk) => consumeNativeLog(role, chunk));
  child.on("error", (error) => {
    appendNativeLog(role, error.message || String(error));
  });
  child.on("close", (code, signal) => {
    if (nativeProcesses[role] === child) nativeProcesses[role] = null;
    const exitReason = signal || (code ?? "unknown");
    appendNativeLog(role, `native ${role} exited (${exitReason})`);
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.webContents.send("sanser:native-exit", { role, code, signal, status: nativeStatus() });
    }
  });

  return child;
}

function consumeNativeLog(role, chunk) {
  nativeLogBuffers[role] += chunk.toString("utf8");
  const lines = nativeLogBuffers[role].split(/\r?\n/);
  nativeLogBuffers[role] = lines.pop() || "";
  for (const line of lines) {
    if (line.trim()) appendNativeLog(role, line);
  }
}

function appendNativeLog(role, line) {
  const entry = {
    role,
    line,
    at: Date.now()
  };
  nativeLogs[role].push(entry);
  if (nativeLogs[role].length > 80) nativeLogs[role].shift();
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.webContents.send("sanser:native-log", entry);
  }
}

function stopNativeProcess(role) {
  const child = nativeProcesses[role];
  if (!child) return nativeStatus();
  nativeProcesses[role] = null;
  if (!child.killed) child.kill();
  return nativeStatus();
}

function nativeStatus() {
  return {
    platform: process.platform,
    client: processStatus("client"),
    host: processStatus("host"),
    logs: nativeLogs
  };
}

function nativeNetworkInfo() {
  return {
    platform: process.platform,
    addresses: localNetworkAddresses()
  };
}

function localNetworkAddresses() {
  const interfaces = os.networkInterfaces();
  const addresses = [];
  for (const [name, entries] of Object.entries(interfaces)) {
    for (const entry of entries || []) {
      if (entry.internal || entry.family !== "IPv4") continue;
      addresses.push({
        name,
        address: entry.address,
        netmask: entry.netmask,
        cidr: entry.cidr || "",
        priority: networkAddressPriority(name, entry.address)
      });
    }
  }
  return addresses.sort((a, b) => b.priority - a.priority || a.name.localeCompare(b.name));
}

function networkAddressPriority(name, address) {
  if (/^100\./.test(address)) return 90;
  if (/^(en|eth|wi-fi|wlan|ethernet)/i.test(name)) return 70;
  if (/^192\.168\.|^10\.|^172\.(1[6-9]|2\d|3[0-1])\./.test(address)) return 60;
  if (/^(utun|tailscale)/i.test(name)) return 50;
  return 10;
}

function processStatus(role) {
  const child = nativeProcesses[role];
  return {
    running: Boolean(child && !child.killed),
    pid: child && !child.killed ? child.pid : null
  };
}

function resolveNativeClientPath() {
  return resolveNativeBinary("native/client-mac/build/sanser-native-client");
}

function resolveNativeHostPath() {
  return resolveNativeBinary("native/host-win/build/Release/sanser-native-host.exe");
}

function resolveNativeBinary(relativePath) {
  if (!app.isPackaged) {
    return path.join(__dirname, "..", relativePath);
  }
  return path.join(process.resourcesPath, "app.asar.unpacked", relativePath);
}

function clampInt(value, min, max, fallback) {
  const number = Number(value);
  if (!Number.isFinite(number)) return fallback;
  return Math.max(min, Math.min(max, Math.round(number)));
}

function sendHostInput(payload) {
  if (sendNativeHostInput(payload)) return;

  const type = String(payload.type || "");
  if (type.startsWith("pointer")) {
    const point = screenPoint(payload);
    mainWindow.webContents.sendInputEvent({
      type: "mouseMove",
      x: point.x,
      y: point.y,
      button: mouseButton(payload.button)
    });
    if (type === "pointer-down" || type === "pointer-up") {
      mainWindow.webContents.sendInputEvent({
        type: type === "pointer-down" ? "mouseDown" : "mouseUp",
        x: point.x,
        y: point.y,
        button: mouseButton(payload.button),
        clickCount: 1
      });
    }
    return;
  }

  if (type === "wheel") {
    mainWindow.webContents.sendInputEvent({
      type: "mouseWheel",
      deltaX: Number(payload.dx || 0),
      deltaY: Number(payload.dy || 0)
    });
    return;
  }

  if (type === "key-down" || type === "key-up") {
    mainWindow.webContents.sendInputEvent({
      type: type === "key-down" ? "keyDown" : "keyUp",
      keyCode: String(payload.key || payload.code || ""),
      modifiers: keyModifiers(payload)
    });
  }
}

function sendNativeHostInput(payload) {
  if (process.platform !== "win32") return false;
  const worker = getNativeInputWorker();
  if (!worker?.stdin?.writable) return false;
  const point = screenPoint(payload);
  const nativePayload = {
    ...payload,
    screenX: point.x,
    screenY: point.y,
    buttonName: mouseButton(payload.button)
  };
  worker.stdin.write(`${JSON.stringify(nativePayload)}\n`);
  return true;
}

function getNativeInputWorker() {
  if (nativeInputWorker && !nativeInputWorker.killed) return nativeInputWorker;
  const script = resolveInputWorkerPath();
  nativeInputWorker = spawn("powershell.exe", [
    "-NoProfile",
    "-ExecutionPolicy",
    "Bypass",
    "-File",
    script
  ], {
    stdio: ["pipe", "ignore", "ignore"],
    windowsHide: true
  });
  nativeInputWorker.on("exit", () => {
    nativeInputWorker = null;
  });
  nativeInputWorker.on("error", () => {
    nativeInputWorker = null;
  });
  return nativeInputWorker;
}

function resolveInputWorkerPath() {
  if (!app.isPackaged) return path.join(__dirname, "input-worker-win.ps1");
  return path.join(process.resourcesPath, "app.asar.unpacked", "desktop", "input-worker-win.ps1");
}

function screenPoint(payload) {
  const bounds = targetDisplayBounds(payload);
  return {
    x: Math.round(bounds.x + clamp01(Number(payload.x || 0)) * bounds.width),
    y: Math.round(bounds.y + clamp01(Number(payload.y || 0)) * bounds.height)
  };
}

function targetDisplayBounds(payload) {
  const displays = screen.getAllDisplays();
  const sourceWidth = Number(payload.sourceWidth || 0);
  const sourceHeight = Number(payload.sourceHeight || 0);
  if (sourceWidth > 0 && sourceHeight > 0) {
    const sourceRatio = sourceWidth / sourceHeight;
    const byRatio = displays
      .map((display) => ({
        display,
        diff: Math.abs((display.bounds.width / display.bounds.height) - sourceRatio)
      }))
      .sort((a, b) => a.diff - b.diff)[0];
    if (byRatio) return byRatio.display.bounds;
  }
  return screen.getPrimaryDisplay().bounds;
}

function mouseButton(button) {
  if (button === 1) return "middle";
  if (button === 2) return "right";
  return "left";
}

function keyModifiers(payload) {
  const modifiers = [];
  if (payload.alt) modifiers.push("alt");
  if (payload.ctrl) modifiers.push("control");
  if (payload.shift) modifiers.push("shift");
  if (payload.meta) modifiers.push("meta");
  return modifiers;
}

function clamp01(value) {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(1, value));
}

function installScreenCaptureHandler() {
  if (!session.defaultSession.setDisplayMediaRequestHandler) return;

  session.defaultSession.setDisplayMediaRequestHandler(async (_request, callback) => {
    try {
      const sources = await desktopCapturer.getSources({
        types: ["screen", "window"],
        thumbnailSize: { width: 0, height: 0 }
      });
      const source = sources.find((item) => item.id.startsWith("screen:")) || sources[0];
      if (!source) {
        callback({ useSystemPicker: true });
        return;
      }
      callback({ video: source });
    } catch {
      // Fallback to system picker if getSources fails (common on macOS 14+)
      callback({ useSystemPicker: true });
    }
  }, { useSystemPicker: true });
}
