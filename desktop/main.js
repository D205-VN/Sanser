const path = require("path");
const fs = require("fs");
const { spawn, spawnSync } = require("child_process");
const { app, BrowserWindow, desktopCapturer, dialog, session, shell } = require("electron");
const { startServer } = require("../server");

let mainWindow = null;
let serverHandle = null;

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
});

async function boot() {
  process.env.GAME_REMOTE_DATA_DIR = path.join(app.getPath("userData"), "data");
  await ensureTailscaleInstalled();
  installScreenCaptureHandler();
  serverHandle = await startEmbeddedServer();
  createWindow(`http://127.0.0.1:${serverHandle.port}`);
}

async function ensureTailscaleInstalled() {
  if (process.env.NETWORK_MODE !== "tailscale") return;
  if (isTailscaleInstalled()) {
    openTailscale();
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
    openTailscale();
    dialog.showMessageBox({
      type: "info",
      title: "Tailscale đã sẵn sàng",
      message: "Mở Tailscale và đăng nhập cùng tài khoản trên cả Mac và Windows.",
      buttons: ["OK"]
    });
  } catch (error) {
    dialog.showErrorBox("Không cài được Tailscale", error.message);
  }
}

function isTailscaleInstalled() {
  if (commandExists("tailscale")) return true;
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
      sandbox: true
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
