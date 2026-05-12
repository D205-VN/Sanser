const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("sanserHost", {
  input(payload) {
    ipcRenderer.send("sanser:host-input", payload);
  }
});
