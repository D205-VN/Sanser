const path = require("path");
const fs = require("fs");
const os = require("os");
const dgram = require("dgram");
const crypto = require("crypto");
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
const NATIVE_DISCOVERY_PORT = 47777;
const NATIVE_DISCOVERY_TTL_MS = 12000;
const nativeDiscovery = {
  socket: null,
  timer: null,
  id: "",
  peers: new Map(),
  lastClientOptions: null,
  lastHostOptions: null
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
  stopNativeDiscovery();
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
  startNativeDiscovery();
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
  ipcMain.handle("sanser:native-discovery", () => nativeDiscoveryStatus());
  ipcMain.handle("sanser:native-discovery-refresh", () => {
    sendNativeDiscoveryBeacon("manual");
    return nativeDiscoveryStatus();
  });
  ipcMain.handle("sanser:native-start-client", (_event, options = {}) => startNativeClient(options));
  ipcMain.handle("sanser:native-stop-client", () => stopNativeProcess("client"));
  ipcMain.handle("sanser:native-start-host", (_event, options = {}) => startNativeHost(options));
  ipcMain.handle("sanser:native-stop-host", () => stopNativeProcess("host"));
}

function discoveryIdPath() {
  return path.join(app.getPath("userData"), "native-discovery-id");
}

function ensureNativeDiscoveryId() {
  if (nativeDiscovery.id) return nativeDiscovery.id;
  const file = discoveryIdPath();
  try {
    const saved = fs.readFileSync(file, "utf8").trim();
    if (/^[a-f0-9-]{16,64}$/i.test(saved)) {
      nativeDiscovery.id = saved;
      return nativeDiscovery.id;
    }
  } catch (_) {
    // Missing discovery id is normal on first launch.
  }
  nativeDiscovery.id = crypto.randomUUID ? crypto.randomUUID() : crypto.randomBytes(16).toString("hex");
  try {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, nativeDiscovery.id);
  } catch (error) {
    appendNativeLog("discovery", `id persist failed: ${error.message || error}`);
  }
  return nativeDiscovery.id;
}

function startNativeDiscovery() {
  if (nativeDiscovery.socket) return;
  ensureNativeDiscoveryId();

  const socket = dgram.createSocket({ type: "udp4", reuseAddr: true });
  nativeDiscovery.socket = socket;
  socket.on("error", (error) => {
    appendNativeLog("discovery", `UDP discovery error: ${error.message || error}`);
  });
  socket.on("message", (message, remote) => {
    handleNativeDiscoveryMessage(message, remote);
  });
  socket.bind(NATIVE_DISCOVERY_PORT, "0.0.0.0", () => {
    try {
      socket.setBroadcast(true);
    } catch (error) {
      appendNativeLog("discovery", `broadcast disabled: ${error.message || error}`);
    }
    sendNativeDiscoveryBeacon("startup");
    nativeDiscovery.timer = setInterval(() => sendNativeDiscoveryBeacon("interval"), 2000);
    appendNativeLog("discovery", `LAN discovery listening udp:${NATIVE_DISCOVERY_PORT}`);
  });
}

function stopNativeDiscovery() {
  if (nativeDiscovery.timer) clearInterval(nativeDiscovery.timer);
  nativeDiscovery.timer = null;
  if (nativeDiscovery.socket) {
    try {
      nativeDiscovery.socket.close();
    } catch (_) {
      // Socket may already be closed during app shutdown.
    }
  }
  nativeDiscovery.socket = null;
}

function nativeDiscoveryPayload(reason) {
  const clientOptions = nativeDiscovery.lastClientOptions || {};
  const hostOptions = nativeDiscovery.lastHostOptions || {};
  return {
    magic: "SANSER_DISCOVERY_V1",
    version: 1,
    id: ensureNativeDiscoveryId(),
    reason,
    hostname: os.hostname(),
    platform: process.platform,
    app: app.getName(),
    addresses: localNetworkAddresses(),
    native: {
      clientRunning: Boolean(nativeProcesses.client && !nativeProcesses.client.killed),
      hostRunning: Boolean(nativeProcesses.host && !nativeProcesses.host.killed),
      clientPort: Number(clientOptions.port || 0),
      controlPort: Number(clientOptions.controlPort || 0),
      audioPort: Number(clientOptions.audioPort || 0),
      hostEndpoint: String(hostOptions.endpoint || ""),
      hostControlEndpoint: String(hostOptions.controlEndpoint || ""),
      hostAudioEndpoint: String(hostOptions.audioEndpoint || ""),
      videoTransport: String(clientOptions.videoTransport || hostOptions.videoTransport || "udp")
    },
    sentAt: Date.now()
  };
}

function broadcastTargets() {
  const targets = new Set(["255.255.255.255"]);
  for (const entry of localNetworkAddresses()) {
    const address = ipv4ToInt(entry.address);
    const mask = ipv4ToInt(entry.netmask);
    if (address === null || mask === null) continue;
    const broadcast = (address | (~mask >>> 0)) >>> 0;
    targets.add(intToIpv4(broadcast));
  }
  return Array.from(targets);
}

function sendNativeDiscoveryBeacon(reason = "manual") {
  const socket = nativeDiscovery.socket;
  if (!socket) return;
  const payload = Buffer.from(JSON.stringify(nativeDiscoveryPayload(reason)));
  for (const target of broadcastTargets()) {
    socket.send(payload, 0, payload.length, NATIVE_DISCOVERY_PORT, target, (error) => {
      if (error && reason === "manual") {
        appendNativeLog("discovery", `beacon failed ${target}: ${error.message || error}`);
      }
    });
  }
}

function handleNativeDiscoveryMessage(message, remote) {
  let payload = null;
  try {
    payload = JSON.parse(message.toString("utf8"));
  } catch (_) {
    return;
  }
  if (payload?.magic !== "SANSER_DISCOVERY_V1" || !payload.id || payload.id === nativeDiscovery.id) return;

  const peer = {
    id: String(payload.id),
    hostname: String(payload.hostname || ""),
    platform: String(payload.platform || ""),
    app: String(payload.app || ""),
    remoteAddress: remote.address,
    addresses: Array.isArray(payload.addresses) ? payload.addresses : [],
    native: payload.native || {},
    lastSeenAt: Date.now()
  };
  nativeDiscovery.peers.set(peer.id, peer);
  if (mainWindow && !mainWindow.isDestroyed()) {
    mainWindow.webContents.send("sanser:native-discovery", nativeDiscoveryStatus());
  }
}

function nativeDiscoveryStatus() {
  pruneNativeDiscoveryPeers();
  return {
    id: nativeDiscovery.id,
    port: NATIVE_DISCOVERY_PORT,
    peers: Array.from(nativeDiscovery.peers.values())
      .sort((a, b) => b.lastSeenAt - a.lastSeenAt)
  };
}

function pruneNativeDiscoveryPeers() {
  const now = Date.now();
  for (const [id, peer] of nativeDiscovery.peers) {
    if (now - Number(peer.lastSeenAt || 0) > NATIVE_DISCOVERY_TTL_MS) {
      nativeDiscovery.peers.delete(id);
    }
  }
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

  const port = clampInt(options.port, 1, 65533, 7777);
  const controlPort = clampInt(options.controlPort, 0, 65535, port < 65535 ? port + 1 : 0);
  const audioPort = clampInt(options.audioPort, 0, 65535, port < 65534 ? port + 2 : 0);
  const audioJitterMs = clampInt(options.audioJitterMs, 0, 120, 24);
  const audioDevice = String(options.audioDevice || "").trim();
  const audioVolume = clampNumber(options.audioVolume, 0, 1, 1);
  const audioMuted = options.audioMuted === true;
  const videoTransport = String(options.videoTransport || "udp").toLowerCase();
  const maxPackets = clampInt(options.maxPackets, 0, Number.MAX_SAFE_INTEGER, 0);
  const sessionToken = sanitizeNativeSessionToken(options.sessionToken);
  nativeDiscovery.lastClientOptions = { ...options, port, controlPort, audioPort, videoTransport };
  const args = ["--listen-render-snv", String(port)];
  if (controlPort > 0) args.push("--control-port", String(controlPort));
  if (audioPort > 0) args.push("--audio-port", String(audioPort));
  if (audioPort > 0) args.push("--audio-jitter-ms", String(audioJitterMs));
  if (audioPort > 0 && audioDevice) args.push("--audio-device", audioDevice);
  if (audioPort > 0) args.push("--audio-volume", audioVolume.toFixed(3));
  if (audioPort > 0 && audioMuted) args.push("--audio-muted");
  if (videoTransport === "udp") args.push("--udp-video");
  if (maxPackets > 0) args.push("--max-packets", String(maxPackets));
  if (options.logInput === true) args.push("--log-input");
  if (options.fullscreen !== false) args.push("--fullscreen");
  if (options.hideCursor !== false) args.push("--hide-cursor");
  if (options.relativeMouse !== false) args.push("--relative-mouse");

  nativeProcesses.client = spawnNativeProcess(
    "client",
    executable,
    args,
    sessionToken ? { SANSER_NATIVE_SESSION_TOKEN: sessionToken } : {}
  );
  sendNativeDiscoveryBeacon("client-start");
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
  const controlEndpoint = String(options.controlEndpoint || "").trim();
  if (controlEndpoint && !/^\[?[A-Za-z0-9:._-]+\]?:\d+$/.test(controlEndpoint)) {
    throw new Error("Native host cần controlEndpoint dạng HOST:PORT.");
  }
  const audioEndpoint = String(options.audioEndpoint || "").trim();
  if (audioEndpoint && !/^\[?[A-Za-z0-9:._-]+\]?:\d+$/.test(audioEndpoint)) {
    throw new Error("Native host cần audioEndpoint dạng HOST:PORT.");
  }

  stopNativeProcess("host");
  const executable = resolveNativeHostPath();
  if (!fs.existsSync(executable)) {
    throw new Error(`Không tìm thấy sanser-native-host.exe tại ${executable}. Hãy build bằng npm run native:host-win:build.`);
  }

  const fps = clampInt(options.fps, 30, 120, 60);
  const bitrateMbps = clampInt(options.bitrateMbps, 4, 120, 28);
  const keyframeInterval = clampInt(options.keyframeInterval, 1, 10, 1);
  const videoTransport = String(options.videoTransport || "udp").toLowerCase();
  const videoCodec = normalizeNativeVideoCodec(options.videoCodec);
  const encoderPreference = String(options.encoderPreference || "auto").toLowerCase();
  const sessionToken = sanitizeNativeSessionToken(options.sessionToken);
  nativeDiscovery.lastHostOptions = { ...options, fps, bitrateMbps, keyframeInterval, videoTransport, videoCodec };
  const connectFlag = videoTransport === "udp" ? "--udp-connect" : "--tcp-connect";
  const args = [
    "--encode-pipe",
    videoCodec,
    "--fps",
    String(fps),
    "--interval-ms",
    "0",
    "--bitrate",
    String(bitrateMbps * 1000000),
    "--keyframe-interval",
    String(keyframeInterval),
    connectFlag,
    endpoint
  ];
  if (controlEndpoint) args.push("--control-connect", controlEndpoint);
  if (audioEndpoint) args.push("--audio-udp-connect", audioEndpoint);
  if (/^(auto|nvenc|amf|qsv|mf|software)$/.test(encoderPreference)) {
    args.push("--encoder", encoderPreference);
  }
  if (videoTransport === "udp" && options.udpPacing === false) args.push("--no-udp-pacing");
  if (options.lowLatencyEncoder !== false) args.push("--low-latency-encoder");
  else args.push("--no-low-latency-encoder");
  if (options.softwareEncoder) args.push("--software-encoder");

  nativeProcesses.host = spawnNativeProcess(
    "host",
    executable,
    args,
    sessionToken ? { SANSER_NATIVE_SESSION_TOKEN: sessionToken } : {}
  );
  sendNativeDiscoveryBeacon("host-start");
  return nativeStatus();
}

function spawnNativeProcess(role, executable, args, env = {}) {
  nativeLogs[role] = [];
  nativeLogBuffers[role] = "";
  appendNativeLog(role, `$ ${path.basename(executable)} ${args.join(" ")}`);

  const child = spawn(executable, args, {
    cwd: path.dirname(executable),
    env: { ...process.env, ...env },
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

function sanitizeNativeSessionToken(value) {
  const token = String(value || "").trim();
  return /^[A-Za-z0-9._~-]{16,256}$/.test(token) ? token : "";
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
  if (!nativeLogs[role]) nativeLogs[role] = [];
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
  sendNativeDiscoveryBeacon(`${role}-stop`);
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

function ipv4ToInt(value) {
  const parts = String(value || "").split(".");
  if (parts.length !== 4) return null;
  let result = 0;
  for (const part of parts) {
    if (!/^\d+$/.test(part)) return null;
    const number = Number(part);
    if (number < 0 || number > 255) return null;
    result = (result << 8) | number;
  }
  return result >>> 0;
}

function intToIpv4(value) {
  const number = Number(value) >>> 0;
  return [
    (number >>> 24) & 255,
    (number >>> 16) & 255,
    (number >>> 8) & 255,
    number & 255
  ].join(".");
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

function clampNumber(value, min, max, fallback) {
  const number = Number(value);
  if (!Number.isFinite(number)) return fallback;
  return Math.max(min, Math.min(max, number));
}

function normalizeNativeVideoCodec(value) {
  const codec = String(value || "h264").toLowerCase();
  if (codec === "hevc" || codec === "h265" || codec === "h.265") return "hevc";
  return "h264";
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
