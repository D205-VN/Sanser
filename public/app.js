const appState = {
  token: localStorage.getItem("gr_token") || "",
  user: null,
  apiBase: normalizeServerUrl(localStorage.getItem("gr_server_url") || ""),
  sessionId: localStorage.getItem("gr_session_id") || makeId("sess"),
  deviceId: localStorage.getItem("gr_device_id") || makeId("dev"),
  events: null,
  devices: [],
  hostDevice: null,
  localStream: null,
  localStreamQuality: null,
  hostPeers: new Map(),
  clientPeer: null,
  clientRoom: null,
  inputChannel: null,
  realtimeChannel: null,
  nativeClientConnected: false,
  nativeHostRooms: new Set(),
  pendingRequests: new Map(),
  heartbeatTimer: null,
  mouseTimer: 0,
  iceServers: [
    { urls: "stun:stun.l.google.com:19302" }
  ],
  iceTransportPolicy: "all",
  hasTurn: false
};

const TRANSPORT_FEEDBACK_TYPE = "__sanser_transport_feedback";
const TRANSPORT_ADJUST_INTERVAL_MS = 1800;
const NATIVE_TRANSPORT = "native-snv";
const NATIVE_DEFAULT_PORT = 7777;

localStorage.setItem("gr_session_id", appState.sessionId);
localStorage.setItem("gr_device_id", appState.deviceId);

const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => Array.from(document.querySelectorAll(selector));

// ─── TOAST SYSTEM ───
function showToast(message, type = 'error', duration = 5000) {
  const container = $('#toastContainer');
  if (!container) return;
  const icons = { success: '✓', error: '✕', warning: '⚠' };
  const toast = document.createElement('div');
  toast.className = `toast toast-${type}`;
  toast.innerHTML = `<span class="toast-icon">${icons[type] || ''}</span><span>${escapeHtml(message)}</span><button class="toast-close" type="button">×</button>`;
  toast.querySelector('.toast-close').addEventListener('click', () => removeToast(toast));
  container.appendChild(toast);
  setTimeout(() => removeToast(toast), duration);
}
function removeToast(toast) {
  if (!toast.parentNode) return;
  toast.classList.add('removing');
  setTimeout(() => toast.remove(), 300);
}
function showConnecting(name) {
  const o = $('#connectingOverlay');
  if (o) { o.classList.remove('is-hidden'); $('#connectingTitle').textContent = 'Đang kết nối...'; $('#connectingDesc').textContent = `Đang chờ tín hiệu từ ${name}`; }
}
function hideConnecting() {
  const o = $('#connectingOverlay');
  if (o) o.classList.add('is-hidden');
}

const authView = $("#authView");
const appShell = $("#appShell");
const authForm = $("#authForm");
const authError = $("#authError");
const loginTab = $("#loginTab");
const registerTab = $("#registerTab");
const authSubmit = $("#authSubmit");
const userLabel = $("#userLabel");
const accountEmail = $("#accountEmail");
const serverDot = $("#serverDot");
const serverState = $("#serverState");
const deviceList = $("#deviceList");
const requestList = $("#requestList");
const connectError = $("#connectError");
const streamStatus = $("#streamStatus");
const remoteVideo = $("#remoteVideo");
const hostPreview = $("#hostPreview");
const emptyStream = $("#emptyStream");
const clientStats = $("#clientStats");
const hostStats = $("#hostStats");
const videoShell = $("#videoShell");

let authMode = "login";

init();

function init() {
  bindUi();
  bindNativeEvents();
  hydrateDefaults();
  applyPerformanceDefaults();
  hydrateSettings();
  if (appState.token) {
    api("/api/me").then(({ user }) => enterApp(user)).catch(() => showAuth());
  } else {
    showAuth();
  }
}

function applyPerformanceDefaults() {
  if (localStorage.getItem("gr_perf_defaults_v6") === "1") return;
  localStorage.setItem("gr_setting_clientResolution", "1080p");
  localStorage.setItem("gr_setting_clientFps", "60");
  localStorage.setItem("gr_setting_clientBitrate", "28");
  localStorage.setItem("gr_setting_hostFps", "60");
  localStorage.setItem("gr_setting_hostBitrate", "28");
  localStorage.setItem("gr_setting_codecPreference", "H264");
  localStorage.setItem("gr_setting_transportMode", NATIVE_TRANSPORT);
  localStorage.setItem("gr_setting_nativeListenPort", String(NATIVE_DEFAULT_PORT));
  localStorage.setItem("gr_perf_defaults_v6", "1");
}

function bindUi() {
  loginTab.addEventListener("click", () => setAuthMode("login"));
  registerTab.addEventListener("click", () => setAuthMode("register"));
  authForm.addEventListener("submit", submitAuth);
  $("#authServerUrlInput").addEventListener("change", syncAuthServerUrl);
  $("#authUseLocalServerButton").addEventListener("click", useLocalServer);
  $("#logoutButton").addEventListener("click", logout);
  $("#accountLogoutButton").addEventListener("click", logout);
  $("#refreshDevices").addEventListener("click", refreshDevices);
  $("#computerSearch").addEventListener("input", renderDevices);
  $("#disconnectButton").addEventListener("click", disconnectClient);
  $("#focusInputButton").addEventListener("click", () => videoShell.focus());
  $("#startHostButton").addEventListener("click", () => startHost({ capture: false }));
  $("#stopHostButton").addEventListener("click", stopHost);
  $("#captureButton").addEventListener("click", () => captureScreen().catch(() => {}));
  $("#applyServerButton").addEventListener("click", applyServerUrl);
  document.addEventListener("keydown", handleDocumentKey, true);
  document.addEventListener("keyup", handleDocumentKey, true);

  $$(".rail-item[data-view]").forEach((button) => {
    button.addEventListener("click", () => selectView(button.dataset.view));
  });

  $$(".settings-tab").forEach((button) => {
    button.addEventListener("click", () => selectSettingsTab(button.dataset.settingsTab));
  });

  [
    "clientResolution",
    "clientFps",
    "clientBitrate",
    "overlayMode",
    "hostEnabled",
    "hostFps",
    "hostBitrate",
    "codecPreference",
    "transportMode",
    "nativeListenPort",
    "mouseRate",
    "disconnectHotkey",
    "overlayHotkey",
    "gamepadMode",
    "vibrationMode",
    "lowLatencyMode",
    "statsMode",
    "autoAccept"
  ].forEach((id) => {
    const input = $(`#${id}`);
    if (!input) return;
    input.addEventListener("change", () => {
      localStorage.setItem(`gr_setting_${id}`, input.value);
      applyLiveSetting(id);
    });
  });

  ["hostName", "hostGpu"].forEach((id) => {
    const input = $(`#${id}`);
    input.addEventListener("change", () => {
      localStorage.setItem(id === "hostName" ? "gr_host_name" : "gr_host_gpu", input.value.trim());
      if (appState.hostDevice) startHost({ capture: false }).catch(() => {});
    });
  });

  videoShell.addEventListener("mousemove", sendPointerMove);
  videoShell.addEventListener("mousedown", (event) => {
    videoShell.focus();
    sendInputEvent(pointerPayload("pointer-down", event));
    event.preventDefault();
  });
  videoShell.addEventListener("mouseup", (event) => {
    sendInputEvent(pointerPayload("pointer-up", event));
    event.preventDefault();
  });
  videoShell.addEventListener("wheel", (event) => {
    sendInputEvent({ type: "wheel", dx: event.deltaX, dy: event.deltaY });
    event.preventDefault();
  }, { passive: false });
  videoShell.addEventListener("keydown", (event) => {
    sendInputEvent(keyPayload(event));
    event.preventDefault();
  });
  videoShell.addEventListener("keyup", (event) => {
    sendInputEvent(keyPayload(event));
    event.preventDefault();
  });
}

function bindNativeEvents() {
  if (!window.sanserNative?.onLog) return;
  window.sanserNative.onLog(handleNativeLog);
  window.sanserNative.onExit(handleNativeExit);
}

function handleNativeLog(entry = {}) {
  const line = String(entry.line || "");
  if (entry.role === "client") {
    if (/SNV1 render client connected/i.test(line)) {
      appState.nativeClientConnected = true;
      hideConnecting();
      if (appState._connectTimeout) clearTimeout(appState._connectTimeout);
      streamStatus.textContent = "Native SNV đang stream";
      emptyStream.classList.add("is-hidden");
      $("#clientRoleBadge").classList.remove("is-hidden");
      showToast("Native stream đã kết nối.", "success", 2500);
    }
    if (/SNINPUT_TX/i.test(line)) {
      streamStatus.textContent = "Native SNV đang stream | input hoạt động";
    }
  }
  if (entry.role === "host") {
    if (/SNV1 H\.264 packet stream/i.test(line)) {
      $("#captureStatus").textContent = "Native host đang encode H.264";
    }
    if (/SNINPUT control backchannel enabled/i.test(line)) {
      hostStats.textContent = "Native SNV | input backchannel hoạt động";
    }
  }
}

function handleNativeExit(payload = {}) {
  if (payload.role === "client" && appState.clientRoom && isNativeRoom(appState.clientRoom)) {
    streamStatus.textContent = "Native renderer đã dừng";
  }
  if (payload.role === "host") {
    appState.nativeHostRooms.clear();
    $("#captureStatus").textContent = "Native host đã dừng";
    hostStats.textContent = "Native SNV inactive";
  }
}

function handleDocumentKey(event) {
  if (!appState.inputChannel || appState.inputChannel.readyState !== "open") return;
  if (isEditableTarget(event.target)) return;
  sendInputEvent(keyPayload(event));
  event.preventDefault();
  event.stopPropagation();
}

function keyPayload(event) {
  const macCommandAsControl = /Mac/i.test(navigator.platform) && event.metaKey;
  return {
    type: event.type === "keydown" ? "key-down" : "key-up",
    key: event.key,
    code: event.code,
    alt: event.altKey,
    ctrl: event.ctrlKey || macCommandAsControl,
    shift: event.shiftKey,
    meta: false
  };
}

function isEditableTarget(target) {
  if (!target) return false;
  const tag = target.tagName;
  return target.isContentEditable || tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
}

function hydrateDefaults() {
  $("#hostName").value = localStorage.getItem("gr_host_name") || defaultComputerName();
  $("#hostGpu").value = localStorage.getItem("gr_host_gpu") || defaultGpuLabel();
  $("#serverUrlInput").value = appState.apiBase;
  $("#authServerUrlInput").value = appState.apiBase;
  updateServerUrlLabel();
}

function hydrateSettings() {
  $$("[id]").forEach((node) => {
    if (!["INPUT", "SELECT"].includes(node.tagName)) return;
    const saved = localStorage.getItem(`gr_setting_${node.id}`);
    if (saved !== null) node.value = saved;
  });
  normalizeTransportSetting();
  applyLiveSetting("statsMode");
}

function normalizeTransportSetting() {
  const select = $("#transportMode");
  if (!select) return;
  const saved = localStorage.getItem("gr_setting_transportMode");
  if (!saved || saved === "p2p") {
    select.value = "webrtc-adaptive";
    localStorage.setItem("gr_setting_transportMode", "webrtc-adaptive");
  }
}

function setAuthMode(mode) {
  authMode = mode;
  loginTab.classList.toggle("is-active", mode === "login");
  registerTab.classList.toggle("is-active", mode === "register");
  $$(".register-only").forEach((node) => node.classList.toggle("is-hidden", mode !== "register"));
  authSubmit.textContent = mode === "login" ? "Đăng nhập" : "Tạo tài khoản";
  authError.textContent = "";
}

async function submitAuth(event) {
  event.preventDefault();
  authError.textContent = "";
  syncAuthServerUrl();
  authSubmit.disabled = true;
  authSubmit.textContent = authMode === "login" ? "Đang đăng nhập..." : "Đang tạo tài khoản...";
  const body = {
    name: $("#nameInput").value.trim(),
    email: $("#emailInput").value.trim(),
    password: $("#passwordInput").value
  };
  try {
    const data = await api(authMode === "login" ? "/api/login" : "/api/register", { method: "POST", body });
    appState.token = data.token;
    localStorage.setItem("gr_token", data.token);
    enterApp(data.user);
  } catch (error) {
    const message = friendlyAuthError(error.message);
    authError.textContent = message;
    showToast(message, "error");
  } finally {
    authSubmit.disabled = false;
    authSubmit.textContent = authMode === "login" ? "Đăng nhập" : "Tạo tài khoản";
  }
}

function friendlyAuthError(message) {
  if (/Invalid email or password/i.test(message)) return "Email hoặc mật khẩu không đúng.";
  if (/Email already exists/i.test(message)) return "Email này đã được đăng ký.";
  if (/Name, email/i.test(message)) return "Vui lòng nhập tên, email và mật khẩu ít nhất 6 ký tự.";
  if (/Failed to fetch|NetworkError|Load failed|AbortError|Request timed out/i.test(message)) {
    return `Không kết nối được server (${currentServerLabel()}). Kiểm tra Server URL hoặc mạng.`;
  }
  return message || "Đăng nhập thất bại.";
}

function showAuth() {
  authView.classList.remove("is-hidden");
  appShell.classList.add("is-hidden");
}

function enterApp(user) {
  appState.user = user;
  userLabel.textContent = accountHandle(user);
  accountEmail.textContent = user.email;
  authView.classList.add("is-hidden");
  appShell.classList.remove("is-hidden");
  loadNetworkConfig().catch(() => {});
  connectEvents();
  refreshDevices();
  maybeAutoHost();
}

async function loadNetworkConfig() {
  const config = await api("/api/config");
  if (Array.isArray(config.iceServers) && config.iceServers.length) {
    appState.iceServers = config.iceServers;
  }
  appState.hasTurn = Boolean(config.hasTurn);
  appState.iceTransportPolicy = config.iceTransportPolicy === "relay" ? "relay" : "all";
}

async function logout() {
  try {
    await api("/api/logout", { method: "POST" });
  } catch {
    // Token may already be invalid.
  }
  await stopHost();
  localStorage.removeItem("gr_token");
  appState.token = "";
  if (appState.events) appState.events.close();
  showAuth();
}

function selectView(viewId) {
  $$(".rail-item[data-view]").forEach((button) => button.classList.toggle("is-active", button.dataset.view === viewId));
  $$(".view").forEach((view) => view.classList.toggle("is-active", view.id === viewId));
}

function selectSettingsTab(tabId) {
  $$(".settings-tab").forEach((button) => button.classList.toggle("is-active", button.dataset.settingsTab === tabId));
  $$(".settings-pane").forEach((pane) => pane.classList.toggle("is-active", pane.id === tabId));
}

function connectEvents() {
  if (appState.events) appState.events.close();
  const events = new EventSource(apiUrl(`/api/events?token=${encodeURIComponent(appState.token)}`));
  appState.events = events;

  events.addEventListener("ready", () => setServerState(true));
  events.addEventListener("ping", () => setServerState(true));
  events.addEventListener("error", () => setServerState(false));
  events.addEventListener("devices", (event) => {
    appState.devices = JSON.parse(event.data).devices || [];
    renderDevices();
  });
  events.addEventListener("connect-request", (event) => {
    const data = JSON.parse(event.data);
    if (data.targetSessionId !== appState.sessionId) return;
    appState.pendingRequests.set(data.room.id, data.room);
    renderRequests();
  });
  events.addEventListener("connect-accepted", async (event) => {
    const data = JSON.parse(event.data);
    if (data.targetSessionId !== appState.sessionId) return;
    if (isNativeRoom(data.room)) {
      if (data.room.hostSessionId === appState.sessionId) {
        await startNativeHostForRoom(data.room).catch((error) => {
          $("#captureStatus").textContent = error.message || "Could not start native host";
          api("/api/connect/close", { method: "POST", body: { roomId: data.room.id } }).catch(() => {});
        });
        return;
      }
      handleNativeClientAccepted(data.room);
      return;
    }
    if (data.room.hostSessionId === appState.sessionId) {
      await startHostPeer(data.room).catch((error) => {
        $("#captureStatus").textContent = error.message || "Could not start stream";
        api("/api/connect/close", { method: "POST", body: { roomId: data.room.id } }).catch(() => {});
      });
      return;
    }
    await startClientPeer(data.room);
    streamStatus.textContent = "Waiting for host offer";
  });
  events.addEventListener("connect-rejected", (event) => {
    const data = JSON.parse(event.data);
    if (data.targetSessionId !== appState.sessionId) return;
    streamStatus.textContent = "Connection rejected";
  });
  events.addEventListener("connect-closed", (event) => {
    const data = JSON.parse(event.data);
    if (data.targetSessionId !== appState.sessionId) return;
    if (isNativeRoom(data.room)) {
      handleNativeClosed(data.room);
      return;
    }
    if (appState.hostPeers.has(data.room.id)) {
      appState.hostPeers.get(data.room.id).close();
      appState.hostPeers.delete(data.room.id);
      $("#captureStatus").textContent = "Client disconnected";
      return;
    }
    cleanupClientPeer();
  });
  events.addEventListener("signal", (event) => {
    const data = JSON.parse(event.data);
    if (data.targetSessionId !== appState.sessionId) return;
    handleSignal(data);
  });
}

function setServerState(online) {
  serverDot.classList.toggle("online", online);
  serverState.textContent = online ? "Online" : "Reconnecting";
}

async function refreshDevices() {
  const data = await api("/api/devices");
  appState.devices = data.devices || [];
  renderDevices();
}

function renderDevices() {
  const query = ($("#computerSearch").value || "").trim().toLowerCase();
  const devices = appState.devices.filter((device) => {
    const haystack = `${device.name || ""} ${device.gpu || ""} ${device.ip || ""} ${device.platform || ""}`.toLowerCase();
    return !query || haystack.includes(query);
  });

  if (!devices.length) {
    deviceList.innerHTML = `
      <article class="computer-card is-offline">
        <div class="computer-illustration">${computerArt()}</div>
        <h2 class="computer-name">Chưa có máy tính</h2>
        <p class="computer-meta">Cùng tài khoản</p>
        <div class="computer-spec">Không tìm thấy host online.</div>
        <button class="connect-card-button" disabled type="button">Kết nối</button>
      </article>
    `;
    return;
  }

  deviceList.innerHTML = "";
  for (const device of devices) {
    const isSelf = device.sessionId === appState.sessionId;
    const buttonText = isSelf ? "Máy này" : "Kết nối";
    const disabled = !device.online || isSelf;
    const statusClass = device.online ? "is-online" : "is-offline";
    const statusText = device.online ? "Online" : "Offline";
    const ipDisplay = device.ip ? `<span class="computer-ip">${escapeHtml(device.ip)}</span>` : '';
    const card = document.createElement("article");
    card.className = `computer-card ${device.online ? "" : "is-offline"} ${isSelf ? "is-self" : ""}`;
    card.innerHTML = `
      <div class="computer-illustration">${computerArt()}</div>
      <h2 class="computer-name">${escapeHtml(device.name || "Gaming PC")}</h2>
      <span class="online-badge ${statusClass}"><span class="dot"></span>${statusText}</span>
      ${ipDisplay}
      <div class="computer-spec">${escapeHtml(device.gpu || "Hardware encoder")} · ${escapeHtml(device.quality?.label || "1080p")}</div>
      <button class="connect-card-button" ${disabled ? "disabled" : ""} type="button">${buttonText}</button>
    `;
    card.querySelector("button").addEventListener("click", () => connectToDevice(device.id));
    deviceList.appendChild(card);
  }
}

function computerArt() {
  return `<span class="pc-icon"><svg viewBox="0 0 24 24"><path d="M4 5h16v10H4V5Zm2 2v6h12V7H6Zm2 11h8v2H8v-2Zm-3 2h14v2H5v-2Z"/></svg></span>`;
}

async function maybeAutoHost() {
  if ($("#hostEnabled").value !== "on") return;
  await startHost({ capture: false }).catch(() => {});
}

async function startHost(options = {}) {
  const capture = options.capture === true;
  localStorage.setItem("gr_host_name", $("#hostName").value.trim());
  localStorage.setItem("gr_host_gpu", $("#hostGpu").value.trim());
  const quality = readQuality();
  const data = await api("/api/host/online", {
    method: "POST",
    body: {
      sessionId: appState.sessionId,
      deviceId: appState.deviceId,
      name: $("#hostName").value.trim() || defaultComputerName(),
      gpu: $("#hostGpu").value.trim() || defaultGpuLabel(),
      platform: navigator.userAgent,
      autoAccept: $("#autoAccept").value === "on",
      quality
    }
  });
  appState.hostDevice = data.device;
  $("#hostStatus").textContent = "Online";
  $("#hostRoleBadge").classList.remove("is-hidden");
  startHeartbeat();
  if (capture) {
    await captureScreen().catch((error) => {
      $("#captureStatus").textContent = error.message || "Screen capture cancelled";
    });
  }
  return data.device;
}

function startHeartbeat() {
  if (appState.heartbeatTimer) clearInterval(appState.heartbeatTimer);
  appState.heartbeatTimer = setInterval(() => {
    if (!appState.hostDevice) return;
    api("/api/host/heartbeat", {
      method: "POST",
      body: { deviceId: appState.hostDevice.id, status: appState.localStream ? "capturing" : "ready" }
    }).catch(() => {});
  }, 8000);
}

async function stopHost() {
  if (appState.heartbeatTimer) clearInterval(appState.heartbeatTimer);
  appState.heartbeatTimer = null;
  if (window.sanserNative?.stopHost) {
    await window.sanserNative.stopHost().catch(() => {});
  }
  appState.nativeHostRooms.clear();
  if (appState.hostDevice) {
    await api("/api/host/offline", { method: "POST", body: { deviceId: appState.hostDevice.id } }).catch(() => {});
  }
  for (const peer of appState.hostPeers.values()) peer.close();
  appState.hostPeers.clear();
  stopLocalStream();
  appState.hostDevice = null;
  $("#hostStatus").textContent = "Offline";
  $("#hostRoleBadge").classList.add("is-hidden");
  $("#captureStatus").textContent = "Screen capture inactive";
}

async function captureScreen(qualityOverride = null) {
  if (!navigator.mediaDevices?.getDisplayMedia) {
    $("#captureStatus").textContent = "Screen capture is not supported in this runtime";
    return null;
  }
  const quality = qualityOverride || appState.hostDevice?.quality || readQuality();
  stopLocalStream();
  const video = {
    width: { ideal: quality.width },
    height: { ideal: quality.height },
    frameRate: { ideal: quality.fps, max: quality.fps },
    cursor: "always"
  };
  try {
    appState.localStream = await navigator.mediaDevices.getDisplayMedia({
      video,
      audio: {
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false
      }
    });
  } catch (error) {
    if (error.name === "NotAllowedError") {
      $("#captureStatus").textContent = "Screen capture cancelled";
      throw error;
    }
    appState.localStream = await navigator.mediaDevices.getDisplayMedia({ video, audio: false });
  }
  hostPreview.srcObject = appState.localStream;
  for (const track of appState.localStream.getVideoTracks()) {
    track.contentHint = "motion";
  }
  appState.localStreamQuality = quality;
  $("#captureStatus").textContent = `${quality.width}x${quality.height} @ ${quality.fps} FPS`;
  return appState.localStream;
}

function stopLocalStream() {
  if (!appState.localStream) return;
  appState.localStream.getTracks().forEach((track) => track.stop());
  appState.localStream = null;
  appState.localStreamQuality = null;
  hostPreview.srcObject = null;
}

function renderRequests() {
  if (!appState.pendingRequests.size) {
    requestList.innerHTML = `<article class="request-card">No active connection requests.</article>`;
    return;
  }
  requestList.innerHTML = "";
  for (const room of appState.pendingRequests.values()) {
    const card = document.createElement("article");
    card.className = "request-card";
    card.textContent = `${room.clientName} connected at ${room.quality.width}x${room.quality.height} ${room.quality.fps} FPS`;
    requestList.appendChild(card);
  }
}

async function connectToDevice(deviceId) {
  connectError.textContent = "";
  const device = appState.devices.find((item) => item.id === deviceId);
  if (!device || !device.online || device.sessionId === appState.sessionId) return;
  if (isNativeTransport()) {
    await connectNativeToDevice(device);
    return;
  }

  try {
    await loadNetworkConfig().catch(() => {});
    showConnecting(device.name || 'máy chủ');
    streamStatus.textContent = "Đang kết nối...";
    $("#selectedDeviceLabel").textContent = `${device.name} đã chọn`;

    // Connection timeout
    appState._connectTimeout = setTimeout(() => {
      if (!appState.clientPeer || appState.clientPeer.connectionState !== 'connected') {
        hideConnecting();
        showToast(`Không thể kết nối tới ${device.name}. Hết thời gian chờ!`, 'error');
        cleanupClientPeer();
      }
    }, 15000);

    const data = await api("/api/connect/request", {
      method: "POST",
      body: {
        sessionId: appState.sessionId,
        deviceId: device.id,
        quality: readClientQuality()
      }
    });
    appState.clientRoom = data.room;
  } catch (error) {
    hideConnecting();
    showToast(error.message || 'Kết nối thất bại!', 'error');
    connectError.textContent = error.message;
    streamStatus.textContent = "Chờ kết nối";
    if (appState._connectTimeout) clearTimeout(appState._connectTimeout);
  }
}

async function connectNativeToDevice(device) {
  try {
    if (!window.sanserNative?.startClient) {
      throw new Error("Native launcher chỉ chạy trong app desktop Electron.");
    }

    const port = readNativePort();
    showConnecting(device.name || "máy chủ");
    streamStatus.textContent = `Đang mở native renderer trên cổng ${port}...`;
    $("#selectedDeviceLabel").textContent = `${device.name} đã chọn`;
    appState.nativeClientConnected = false;

    await window.sanserNative.startClient({ port, logInput: true });

    appState._connectTimeout = setTimeout(() => {
      if (!appState.nativeClientConnected) {
        hideConnecting();
        streamStatus.textContent = "Native renderer đang chờ host";
        showToast("Native listener đã mở, nhưng Windows host chưa connect vào.", "warning", 5000);
      }
    }, 15000);

    const data = await api("/api/connect/request", {
      method: "POST",
      body: {
        sessionId: appState.sessionId,
        deviceId: device.id,
        quality: readClientQuality(),
        native: {
          transport: "snv-tcp",
          port
        }
      }
    });
    appState.clientRoom = data.room;
    streamStatus.textContent = `Native renderer chờ ${device.name || "host"} connect`;
    emptyStream.classList.add("is-hidden");
    $(".stream-dock").classList.add("is-visible");
  } catch (error) {
    hideConnecting();
    if (appState._connectTimeout) clearTimeout(appState._connectTimeout);
    if (window.sanserNative?.stopClient) {
      await window.sanserNative.stopClient().catch(() => {});
    }
    showToast(error.message || "Không mở được native renderer.", "error");
    connectError.textContent = error.message || "Native connection failed";
    streamStatus.textContent = "Chờ kết nối";
  }
}

async function startNativeHostForRoom(room) {
  if (!window.sanserNative?.startHost) {
    throw new Error("Native launcher chỉ chạy trong app desktop Electron.");
  }
  if (!appState.hostDevice) {
    await startHost({ capture: false });
  }

  const endpoint = room.native?.clientEndpoint;
  if (!endpoint) throw new Error("Native room thiếu endpoint của Mac client.");

  appState.nativeHostRooms.add(room.id);
  $("#captureStatus").textContent = `Native host đang connect ${endpoint}`;
  $("#hostRoleBadge").classList.remove("is-hidden");
  hostStats.textContent = "Native SNV starting";
  await window.sanserNative.startHost({
    endpoint,
    fps: room.quality?.fps || Number($("#hostFps").value || 60),
    bitrateMbps: room.quality?.bitrateMbps || Number($("#hostBitrate").value || 28)
  });
}

function handleNativeClientAccepted(room) {
  appState.clientRoom = room;
  streamStatus.textContent = "Native renderer đang chờ Windows host";
  $(".stream-dock").classList.add("is-visible");
  emptyStream.classList.add("is-hidden");
}

function handleNativeClosed(room) {
  if (room.hostSessionId === appState.sessionId) {
    appState.nativeHostRooms.delete(room.id);
    if (!appState.nativeHostRooms.size) {
      if (window.sanserNative?.stopHost) window.sanserNative.stopHost().catch(() => {});
      $("#captureStatus").textContent = "Native client disconnected";
      hostStats.textContent = "Native SNV inactive";
    }
    return;
  }
  cleanupClientPeer();
}

async function startHostPeer(room) {
  if (appState.hostPeers.has(room.id)) return;
  await loadNetworkConfig().catch(() => {});
  if (!appState.hostDevice) {
    await startHost({ capture: false });
  }
  if (!appState.localStream || !streamMatchesQuality(appState.localStreamQuality, room.quality)) {
    await captureScreen(room.quality);
  }
  const pc = createPeerConnection(room, "host");
  appState.hostPeers.set(room.id, pc);
  const inputChannel = pc.createDataChannel("input", {
    ordered: true,
    maxRetransmits: 3
  });
  inputChannel.onmessage = (event) => handleHostDataMessage(event.data, pc);
  const realtimeChannel = pc.createDataChannel("realtime", {
    ordered: false,
    maxRetransmits: 0
  });
  realtimeChannel.onmessage = (event) => handleHostDataMessage(event.data, pc);
  pc._inputChannel = inputChannel;
  pc._realtimeChannel = realtimeChannel;

  const videoTrack = appState.localStream.getVideoTracks()[0];
  if (videoTrack) {
    const transceiver = pc.addTransceiver(videoTrack, { direction: "sendonly", streams: [appState.localStream] });
    setCodecPreference(transceiver, room.quality.preferCodec || $("#codecPreference").value);
    await tuneSender(transceiver.sender, room.quality);
    attachHostTransport(pc, transceiver.sender, room.quality);
  }
  for (const audioTrack of appState.localStream.getAudioTracks()) {
    pc.addTrack(audioTrack, appState.localStream);
  }
  const offer = await pc.createOffer({
    offerToReceiveAudio: false,
    offerToReceiveVideo: false
  });
  await pc.setLocalDescription(offer);
  await sendSignal(room.id, { type: "offer", description: pc.localDescription });
  monitorHostStats(pc);
}

async function startClientPeer(room) {
  cleanupClientPeer();
  appState.clientRoom = room;
  appState.clientPeer = createPeerConnection(room, "client");
  appState.clientPeer.ontrack = async (event) => {
    hideConnecting();
    if (appState._connectTimeout) clearTimeout(appState._connectTimeout);
    remoteVideo.srcObject = event.streams[0];
    emptyStream.classList.add("is-hidden");
    $(".stream-dock").classList.add("is-visible");
    streamStatus.textContent = "Đang stream";
    $("#clientRoleBadge").classList.remove("is-hidden");
    showToast('Kết nối thành công! Đang stream...', 'success');

    // Fullscreen immediately on stream arrival
    try {
      const vs = document.getElementById("videoShell");
      if (vs.requestFullscreen) await vs.requestFullscreen();
      else if (vs.webkitRequestFullscreen) await vs.webkitRequestFullscreen();
    } catch (e) { console.warn("Fullscreen error:", e); }

    monitorClientStats(appState.clientPeer);
  };
  appState.clientPeer.ondatachannel = (event) => {
    bindClientDataChannel(event.channel);
  };
}

function bindClientDataChannel(channel) {
  if (channel.label === "realtime") {
    appState.realtimeChannel = channel;
  } else {
    appState.inputChannel = channel;
  }

  channel.onopen = () => {
    if (channel.label === "input") {
      streamStatus.textContent = "Đang stream | input hoạt động";
      videoShell.focus();
    }
  };
  channel.onclose = () => {
    if (channel.label === "input") {
      streamStatus.textContent = "Đang stream | input đóng";
    }
    if (channel === appState.inputChannel) appState.inputChannel = null;
    if (channel === appState.realtimeChannel) appState.realtimeChannel = null;
  };
}

function createPeerConnection(room, role) {
  const pc = new RTCPeerConnection({
    bundlePolicy: "max-bundle",
    rtcpMuxPolicy: "require",
    iceTransportPolicy: appState.iceTransportPolicy,
    iceCandidatePoolSize: 4,
    iceServers: appState.iceServers
  });
  pc.pendingRemoteIce = [];
  pc.onicecandidate = (event) => {
    if (event.candidate) sendSignal(room.id, { type: "ice", candidate: event.candidate }).catch(() => {});
  };
  pc.onconnectionstatechange = () => {
    const label = role === "host" ? $("#captureStatus") : streamStatus;
    label.textContent = `${role === "host" ? "Host" : "Client"} ${pc.connectionState}`;
  };
  pc.oniceconnectionstatechange = () => {
    if (role === "client" && ["failed", "disconnected"].includes(pc.iceConnectionState)) {
      hideConnecting();
      if (appState._connectTimeout) clearTimeout(appState._connectTimeout);
      streamStatus.textContent = `Kết nối ${pc.iceConnectionState}`;
      const hint = appState.hasTurn
        ? "Kiểm tra firewall Windows hoặc thử lại."
        : "Nếu khác mạng LAN, cần cấu hình TURN server.";
      showToast(`Không thể kết nối tới máy chủ! ${hint}`, 'error');
      cleanupClientPeer();
    }
  };
  return pc;
}

async function handleSignal(data) {
  const room = appState.clientRoom?.id === data.roomId ? appState.clientRoom : findHostRoom(data.roomId);
  if (!room) return;
  const pc = appState.clientRoom?.id === data.roomId ? appState.clientPeer : appState.hostPeers.get(data.roomId);
  if (!pc) return;
  const message = data.message || {};
  if (message.type === "offer") {
    await pc.setRemoteDescription(message.description);
    await flushRemoteIce(pc);
    const answer = await pc.createAnswer();
    await pc.setLocalDescription(answer);
    await sendSignal(data.roomId, { type: "answer", description: pc.localDescription });
  }
  if (message.type === "answer") {
    await pc.setRemoteDescription(message.description);
    await flushRemoteIce(pc);
  }
  if (message.type === "ice" && message.candidate) {
    await addRemoteIce(pc, message.candidate);
  }
}

async function addRemoteIce(pc, candidate) {
  if (!pc.remoteDescription) {
    pc.pendingRemoteIce.push(candidate);
    return;
  }
  try {
    await pc.addIceCandidate(candidate);
  } catch {
    // Some browsers may emit stale candidates after an ICE restart or close.
  }
}

async function flushRemoteIce(pc) {
  const pending = pc.pendingRemoteIce.splice(0);
  for (const candidate of pending) {
    await addRemoteIce(pc, candidate);
  }
}

function findHostRoom(roomId) {
  if (appState.hostPeers.has(roomId)) return { id: roomId };
  for (const room of appState.pendingRequests.values()) {
    if (room.id === roomId) return room;
  }
  return null;
}

async function sendSignal(roomId, message) {
  await api("/api/signaling", {
    method: "POST",
    body: { roomId, sessionId: appState.sessionId, message }
  });
}

async function tuneSender(sender, quality, overrideBitrateMbps) {
  const bitrateMbps = Number(overrideBitrateMbps || quality.bitrateMbps || 12);
  const params = sender.getParameters();
  params.degradationPreference = $("#lowLatencyMode")?.value === "off" ? "balanced" : "maintain-framerate";
  params.encodings = [{
    maxBitrate: Math.round(bitrateMbps * 1000 * 1000),
    maxFramerate: quality.fps,
    scaleResolutionDownBy: 1,
    priority: "high",
    networkPriority: "high"
  }];
  try {
    await sender.setParameters(params);
  } catch {
    // Browser support for sender knobs varies.
  }
}

function readTransportMode() {
  const raw = $("#transportMode")?.value || "webrtc-adaptive";
  const mode = raw === "p2p" ? "webrtc-adaptive" : raw;
  return {
    mode,
    adaptive: mode === "webrtc-adaptive",
    label: mode === NATIVE_TRANSPORT
      ? "Native SNV"
      : (mode === "webrtc-fixed" ? "WebRTC fixed" : "WebRTC adaptive")
  };
}

function isNativeTransport() {
  return readTransportMode().mode === NATIVE_TRANSPORT;
}

function isNativeRoom(room) {
  return room?.native?.transport === "snv-tcp";
}

function readNativePort() {
  return Math.round(clamp(Number($("#nativeListenPort")?.value || NATIVE_DEFAULT_PORT), 1, 65535));
}

function attachHostTransport(pc, sender, quality) {
  const mode = readTransportMode();
  const maxBitrateMbps = Number(quality.bitrateMbps || 28);
  pc._transport = {
    sender,
    mode: mode.mode,
    adaptive: mode.adaptive,
    quality,
    maxBitrateMbps,
    minBitrateMbps: Math.max(3, Math.round(maxBitrateMbps * 0.25)),
    currentBitrateMbps: maxBitrateMbps,
    stableTicks: 0,
    lastAdjustAt: 0,
    lastReason: mode.adaptive ? "adaptive ready" : "fixed"
  };
}

async function setHostTransportBitrate(pc, nextBitrateMbps, reason) {
  const state = pc._transport;
  if (!state?.sender) return;
  const now = performance.now();
  if (now - state.lastAdjustAt < TRANSPORT_ADJUST_INTERVAL_MS) return;

  const next = clamp(nextBitrateMbps, state.minBitrateMbps, state.maxBitrateMbps);
  if (Math.abs(next - state.currentBitrateMbps) < 0.25) return;

  state.lastAdjustAt = now;
  state.currentBitrateMbps = next;
  state.lastReason = reason;
  await tuneSender(state.sender, state.quality, next).catch(() => {});
}

function updateHostTransportFromFeedback(pc, feedback) {
  const state = pc._transport;
  if (!state?.adaptive) return;

  const rttMs = Number(feedback.rttMs || 0);
  const jitterMs = Number(feedback.jitterMs || 0);
  const lostDelta = Number(feedback.packetsLostDelta || 0);
  const fps = Number(feedback.fps || 0);
  const targetFps = Number(state.quality.fps || 60);

  const severe = rttMs > 220 || jitterMs > 90 || lostDelta > 30 || (fps > 0 && fps < targetFps * 0.55);
  const congested = severe || rttMs > 140 || jitterMs > 45 || lostDelta > 5 || (fps > 0 && fps < targetFps * 0.78);

  if (congested) {
    state.stableTicks = 0;
    const factor = severe ? 0.72 : 0.85;
    setHostTransportBitrate(pc, state.currentBitrateMbps * factor, severe ? "network pressure" : "network adjust");
    return;
  }

  state.stableTicks += 1;
  if (state.stableTicks >= 4 && state.currentBitrateMbps < state.maxBitrateMbps) {
    state.stableTicks = 0;
    setHostTransportBitrate(pc, state.currentBitrateMbps * 1.08, "recovering");
  }
}

function streamMatchesQuality(current, next) {
  if (!current || !next) return false;
  return current.width === next.width
    && current.height === next.height
    && current.fps === next.fps
    && current.bitrateMbps === next.bitrateMbps;
}

function setCodecPreference(transceiver, preferred) {
  if (!RTCRtpSender.getCapabilities || !transceiver.setCodecPreferences) return;
  const caps = RTCRtpSender.getCapabilities("video");
  if (!caps?.codecs?.length) return;
  const target = String(preferred || "VP8").toLowerCase();
  const sorted = [...caps.codecs].sort((a, b) => {
    const aHit = a.mimeType.toLowerCase().includes(target) ? 0 : 1;
    const bHit = b.mimeType.toLowerCase().includes(target) ? 0 : 1;
    return aHit - bHit;
  });
  try {
    transceiver.setCodecPreferences(sorted);
  } catch {
    // Older browser engines may reject codec reordering.
  }
}

function sendPointerMove(event) {
  const now = performance.now();
  const rate = Number($("#mouseRate").value || 60);
  if (now - appState.mouseTimer < 1000 / rate) return;
  appState.mouseTimer = now;
  sendInputEvent(pointerPayload("pointer-move", event));
}

function pointerPayload(type, event) {
  const rect = videoContentRect();
  const x = clamp01((event.clientX - rect.left) / rect.width);
  const y = clamp01((event.clientY - rect.top) / rect.height);
  return {
    type,
    x,
    y,
    button: event.button,
    buttons: event.buttons,
    sourceWidth: remoteVideo.videoWidth || 0,
    sourceHeight: remoteVideo.videoHeight || 0
  };
}

function videoContentRect() {
  const shell = videoShell.getBoundingClientRect();
  const videoWidth = remoteVideo.videoWidth || 16;
  const videoHeight = remoteVideo.videoHeight || 9;
  const videoRatio = videoWidth / videoHeight;
  const shellRatio = shell.width / shell.height;
  let width = shell.width;
  let height = shell.height;
  let left = shell.left;
  let top = shell.top;
  if (shellRatio > videoRatio) {
    width = shell.height * videoRatio;
    left = shell.left + (shell.width - width) / 2;
  } else {
    height = shell.width / videoRatio;
    top = shell.top + (shell.height - height) / 2;
  }
  return { left, top, width, height };
}

function sendInputEvent(payload) {
  const prefersRealtime = payload.type === "pointer-move" || payload.type === "wheel";
  sendDataPayload({ ...payload, at: performance.now() }, prefersRealtime);
}

function sendDataPayload(payload, prefersRealtime = false) {
  const first = prefersRealtime ? appState.realtimeChannel : appState.inputChannel;
  const second = prefersRealtime ? appState.inputChannel : appState.realtimeChannel;
  const channel = [first, second].find((item) => item && item.readyState === "open");
  if (!channel) return;
  channel.send(JSON.stringify(payload));
}

function sendTransportFeedback(stats) {
  sendDataPayload({ type: TRANSPORT_FEEDBACK_TYPE, stats, at: performance.now() }, true);
}

function handleHostDataMessage(raw, pc) {
  try {
    const payload = JSON.parse(raw);
    if (payload.type === TRANSPORT_FEEDBACK_TYPE) {
      updateHostTransportFromFeedback(pc, payload.stats || {});
      return;
    }
    handleRemoteInputPayload(payload);
  } catch {
    $("#captureStatus").textContent = "Input event";
  }
}

function handleRemoteInputPayload(payload) {
  $("#captureStatus").textContent = `Input ${payload.type}`;
  if (window.sanserHost?.input) {
    window.sanserHost.input(payload);
  }
}

function monitorClientStats(pc) {
  pc._stats = pc._stats || {};
  const tick = async () => {
    if (pc !== appState.clientPeer || pc.connectionState === "closed") return;
    const stats = await pc.getStats();
    let fps = "--";
    let bitrate = "--";
    let rtt = "--";
    let jitter = "--";
    let lost = 0;
    let lostDelta = 0;
    let framesDropped = 0;
    stats.forEach((report) => {
      if (report.type === "inbound-rtp" && report.kind === "video") {
        fps = Math.round(report.framesPerSecond || 0);
        lost = report.packetsLost || 0;
        lostDelta = pc._stats.clientLost === undefined ? 0 : Math.max(0, lost - pc._stats.clientLost);
        jitter = report.jitter !== undefined ? Math.round(Number(report.jitter || 0) * 1000) : "--";
        framesDropped = Number(report.framesDropped || 0);
        if (pc._stats.clientBytes && pc._stats.clientAt) {
          const bytes = Number(report.bytesReceived || 0) - pc._stats.clientBytes;
          const seconds = (report.timestamp - pc._stats.clientAt) / 1000;
          bitrate = seconds > 0 ? ((bytes * 8) / seconds / 1000000).toFixed(1) : "--";
        }
        pc._stats.clientBytes = Number(report.bytesReceived || 0);
        pc._stats.clientAt = report.timestamp;
        pc._stats.clientLost = lost;
      }
      if (report.type === "candidate-pair" && report.state === "succeeded" && report.currentRoundTripTime) {
        rtt = Math.round(report.currentRoundTripTime * 1000);
      }
    });
    sendTransportFeedback({
      fps: fps === "--" ? 0 : Number(fps),
      bitrateMbps: bitrate === "--" ? 0 : Number(bitrate),
      rttMs: rtt === "--" ? 0 : Number(rtt),
      jitterMs: jitter === "--" ? 0 : Number(jitter),
      packetsLost: lost,
      packetsLostDelta: lostDelta,
      framesDropped
    });
    clientStats.textContent = `FPS ${fps} | ${bitrate} Mbps | RTT ${rtt} ms | Jitter ${jitter} ms | Lost +${lostDelta}`;
    setTimeout(tick, 1000);
  };
  tick();
}

function monitorHostStats(pc) {
  pc._stats = pc._stats || {};
  const tick = async () => {
    if (pc.connectionState === "closed") return;
    const stats = await pc.getStats();
    let fps = "--";
    let bitrate = "--";
    stats.forEach((report) => {
      if (report.type === "outbound-rtp" && report.kind === "video") {
        fps = Math.round(report.framesPerSecond || report.framesEncoded || 0);
        if (pc._stats.hostBytes && pc._stats.hostAt) {
          const bytes = Number(report.bytesSent || 0) - pc._stats.hostBytes;
          const seconds = (report.timestamp - pc._stats.hostAt) / 1000;
          bitrate = seconds > 0 ? ((bytes * 8) / seconds / 1000000).toFixed(1) : "--";
        }
        pc._stats.hostBytes = Number(report.bytesSent || 0);
        pc._stats.hostAt = report.timestamp;
      }
    });
    const transport = pc._transport;
    const target = transport
      ? `Target ${transport.currentBitrateMbps.toFixed(1)}/${transport.maxBitrateMbps} Mbps | ${transport.lastReason}`
      : "Target --";
    hostStats.textContent = `Outbound FPS ${fps} | ${bitrate} Mbps | ${target}`;
    setTimeout(tick, 1000);
  };
  tick();
}

async function disconnectClient() {
  if (appState.clientRoom) {
    await api("/api/connect/close", { method: "POST", body: { roomId: appState.clientRoom.id } }).catch(() => {});
  }
  cleanupClientPeer();
}

async function cleanupClientPeer() {
  hideConnecting();
  if (appState._connectTimeout) clearTimeout(appState._connectTimeout);
  if (appState.clientRoom && isNativeRoom(appState.clientRoom) && window.sanserNative?.stopClient) {
    await window.sanserNative.stopClient().catch(() => {});
  }
  appState.nativeClientConnected = false;
  if (appState.clientPeer) appState.clientPeer.close();
  appState.clientPeer = null;
  appState.clientRoom = null;
  appState.inputChannel = null;
  appState.realtimeChannel = null;
  remoteVideo.srcObject = null;
  emptyStream.classList.remove("is-hidden");
  $("#clientRoleBadge").classList.add("is-hidden");

  const emptyStrong = document.querySelector("#emptyStream strong");
  const emptySpan = document.querySelector("#emptyStream span");
  if (emptyStrong) emptyStrong.textContent = "Chưa có stream";
  if (emptySpan) emptySpan.textContent = "Chọn một máy tính online để kết nối.";

  streamStatus.textContent = "Chờ kết nối";

  try {
    if (document.fullscreenElement) await document.exitFullscreen();
    else if (document.webkitFullscreenElement) await document.webkitExitFullscreen();
  } catch (e) {}
}

function readQuality() {
  return {
    preset: "1080p",
    width: 1920,
    height: 1080,
    fps: Number($("#hostFps").value || 60),
    bitrateMbps: Number($("#hostBitrate").value || 28),
    preferCodec: $("#codecPreference").value
  };
}

function readClientQuality() {
  const preset = $("#clientResolution").value;
  const presets = {
    "720p": [1280, 720],
    "1080p": [1920, 1080],
    "1440p": [2560, 1440],
    "low-latency": [1600, 900]
  };
  const [width, height] = presets[preset] || presets["1080p"];
  return {
    preset,
    width,
    height,
    fps: Number($("#clientFps").value || 60),
    bitrateMbps: Number($("#clientBitrate").value || 28),
    preferCodec: $("#codecPreference").value
  };
}

function applyLiveSetting(id) {
  if (id === "statsMode" || id === "overlayMode") {
    const hidden = $("#statsMode").value === "off" || $("#overlayMode").value === "off";
    clientStats.classList.toggle("is-hidden", hidden);
  }
  if (id === "hostEnabled") {
    if ($("#hostEnabled").value === "on" && appState.user) {
      startHost({ capture: false }).catch(() => {});
    } else {
      stopHost();
    }
  }
  if (id === "transportMode") {
    const mode = readTransportMode();
    for (const pc of appState.hostPeers.values()) {
      if (!pc._transport) continue;
      pc._transport.mode = mode.mode;
      pc._transport.adaptive = mode.adaptive;
      pc._transport.lastReason = mode.adaptive ? "adaptive ready" : "fixed";
      if (!mode.adaptive) {
        pc._transport.lastAdjustAt = 0;
        setHostTransportBitrate(pc, pc._transport.maxBitrateMbps, "fixed");
      }
    }
  }
}

function applyServerUrl() {
  const nextBase = normalizeServerUrl($("#serverUrlInput").value);
  setServerUrl(nextBase);
  updateServerUrlLabel();
  localStorage.removeItem("gr_token");
  appState.token = "";
  if (appState.events) appState.events.close();
  setServerState(false);
  showAuth();
}

function updateServerUrlLabel() {
  $("#serverUrlLabel").textContent = appState.apiBase
    ? `Using ${appState.apiBase}`
    : "Using local embedded server.";
  $("#authServerHint").textContent = appState.apiBase
    ? `Đang gọi ${appState.apiBase}`
    : `Đang gọi server local (${window.location.origin}).`;
}

function syncAuthServerUrl() {
  const nextBase = normalizeServerUrl($("#authServerUrlInput").value);
  setServerUrl(nextBase);
  $("#serverUrlInput").value = nextBase;
  updateServerUrlLabel();
}

function useLocalServer() {
  $("#authServerUrlInput").value = "";
  syncAuthServerUrl();
  authError.textContent = "";
}

function setServerUrl(nextBase) {
  localStorage.setItem("gr_server_url", nextBase);
  appState.apiBase = nextBase;
}

function currentServerLabel() {
  return appState.apiBase || window.location.origin;
}

async function api(path, options = {}) {
  const headers = { ...(options.headers || {}) };
  if (appState.token) headers.Authorization = `Bearer ${appState.token}`;
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(new Error("Request timed out.")), 12000);
  const init = { ...options, headers, signal: options.signal || controller.signal };
  if (options.body && typeof options.body !== "string") {
    init.body = JSON.stringify(options.body);
    headers["Content-Type"] = "application/json";
  }
  try {
    const response = await fetch(apiUrl(path), init);
    const data = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(data.error || "Request failed.");
    return data;
  } finally {
    clearTimeout(timeout);
  }
}

function apiUrl(path) {
  if (/^https?:\/\//i.test(path)) return path;
  return `${appState.apiBase}${path}`;
}

function normalizeServerUrl(value) {
  const trimmed = String(value || "").trim();
  if (!trimmed) return "";
  return trimmed.replace(/\/+$/, "");
}

function defaultComputerName() {
  const platform = navigator.platform || "Gaming";
  const suffix = appState.deviceId.slice(-6).toUpperCase();
  return `${platform}-${suffix}`;
}

function defaultGpuLabel() {
  if (/Win/i.test(navigator.platform)) return "Windows Hardware Encoder";
  if (/Mac/i.test(navigator.platform)) return "Apple Hardware Encoder";
  return "Hardware Encoder";
}

function accountHandle(user) {
  const base = String(user.name || user.email || "Player").replace(/\s+/g, "");
  const digits = String(hashString(user.id || user.email)).slice(0, 8);
  return `${base}#${digits}`;
}

function hashString(value) {
  let hash = 0;
  for (const char of String(value)) {
    hash = ((hash << 5) - hash + char.charCodeAt(0)) >>> 0;
  }
  return hash;
}

function makeId(prefix) {
  const bytes = new Uint8Array(10);
  crypto.getRandomValues(bytes);
  return `${prefix}_${Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("")}`;
}

function escapeHtml(value) {
  return String(value || "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function clamp01(value) {
  return Math.max(0, Math.min(1, value));
}

function clamp(value, min, max) {
  return Math.max(min, Math.min(max, value));
}
