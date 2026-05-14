const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("sanserHost", {
  input(payload) {
    ipcRenderer.send("sanser:host-input", payload);
  }
});

contextBridge.exposeInMainWorld("sanserNative", {
  status() {
    return ipcRenderer.invoke("sanser:native-status");
  },
  networkInfo() {
    return ipcRenderer.invoke("sanser:native-network-info");
  },
  discovery() {
    return ipcRenderer.invoke("sanser:native-discovery");
  },
  refreshDiscovery() {
    return ipcRenderer.invoke("sanser:native-discovery-refresh");
  },
  startClient(options) {
    return ipcRenderer.invoke("sanser:native-start-client", options || {});
  },
  stopClient() {
    return ipcRenderer.invoke("sanser:native-stop-client");
  },
  startHost(options) {
    return ipcRenderer.invoke("sanser:native-start-host", options || {});
  },
  stopHost() {
    return ipcRenderer.invoke("sanser:native-stop-host");
  },
  onLog(callback) {
    const listener = (_event, payload) => callback(payload);
    ipcRenderer.on("sanser:native-log", listener);
    return () => ipcRenderer.removeListener("sanser:native-log", listener);
  },
  onExit(callback) {
    const listener = (_event, payload) => callback(payload);
    ipcRenderer.on("sanser:native-exit", listener);
    return () => ipcRenderer.removeListener("sanser:native-exit", listener);
  },
  onDiscovery(callback) {
    const listener = (_event, payload) => callback(payload);
    ipcRenderer.on("sanser:native-discovery", listener);
    return () => ipcRenderer.removeListener("sanser:native-discovery", listener);
  }
});
