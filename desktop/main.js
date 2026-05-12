const path = require("path");
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
  installScreenCaptureHandler();
  serverHandle = await startEmbeddedServer();
  createWindow(`http://127.0.0.1:${serverHandle.port}`);
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
        callback({});
        return;
      }
      callback({ video: source });
    } catch {
      callback({});
    }
  }, { useSystemPicker: false });
}
