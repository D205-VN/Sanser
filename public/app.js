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
  hostPeers: new Map(),
  clientPeer: null,
  clientRoom: null,
  inputChannel: null,
  pendingRequests: new Map(),
  heartbeatTimer: null,
  mouseTimer: 0,
  stats: {
    clientBytes: 0,
    clientAt: 0,
    hostBytes: 0,
    hostAt: 0
  }
};

localStorage.setItem("gr_session_id", appState.sessionId);
localStorage.setItem("gr_device_id", appState.deviceId);

const $ = (selector) => document.querySelector(selector);
const $$ = (selector) => Array.from(document.querySelectorAll(selector));

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
  hydrateDefaults();
  hydrateSettings();
  if (appState.token) {
    api("/api/me").then(({ user }) => enterApp(user)).catch(() => showAuth());
  } else {
    showAuth();
  }
}

function bindUi() {
  loginTab.addEventListener("click", () => setAuthMode("login"));
  registerTab.addEventListener("click", () => setAuthMode("register"));
  authForm.addEventListener("submit", submitAuth);
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
    sendInputEvent({
      type: "key-down",
      key: event.key,
      code: event.code,
      alt: event.altKey,
      ctrl: event.ctrlKey,
      shift: event.shiftKey,
      meta: event.metaKey
    });
    event.preventDefault();
  });
  videoShell.addEventListener("keyup", (event) => {
    sendInputEvent({ type: "key-up", key: event.key, code: event.code });
    event.preventDefault();
  });
}

function hydrateDefaults() {
  $("#hostName").value = localStorage.getItem("gr_host_name") || defaultComputerName();
  $("#hostGpu").value = localStorage.getItem("gr_host_gpu") || defaultGpuLabel();
  $("#serverUrlInput").value = appState.apiBase;
  updateServerUrlLabel();
}

function hydrateSettings() {
  $$("[id]").forEach((node) => {
    if (!["INPUT", "SELECT"].includes(node.tagName)) return;
    const saved = localStorage.getItem(`gr_setting_${node.id}`);
    if (saved !== null) node.value = saved;
  });
  applyLiveSetting("statsMode");
}

function setAuthMode(mode) {
  authMode = mode;
  loginTab.classList.toggle("is-active", mode === "login");
  registerTab.classList.toggle("is-active", mode === "register");
  $$(".register-only").forEach((node) => node.classList.toggle("is-hidden", mode !== "register"));
  authSubmit.textContent = mode === "login" ? "Login" : "Create Account";
  authError.textContent = "";
}

async function submitAuth(event) {
  event.preventDefault();
  authError.textContent = "";
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
    authError.textContent = error.message;
  }
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
  connectEvents();
  refreshDevices();
  maybeAutoHost();
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
    const haystack = `${device.name || ""} ${device.gpu || ""} ${device.platform || ""}`.toLowerCase();
    return !query || haystack.includes(query);
  });

  if (!devices.length) {
    deviceList.innerHTML = `
      <article class="computer-card is-offline">
        <div class="computer-illustration">${computerArt()}</div>
        <h2 class="computer-name">No Computers</h2>
        <p class="computer-meta">Same Account</p>
        <div class="computer-spec">No online host found.</div>
        <button class="connect-card-button" disabled type="button">Connect</button>
      </article>
    `;
    return;
  }

  deviceList.innerHTML = "";
  for (const device of devices) {
    const isSelf = device.sessionId === appState.sessionId;
    const buttonText = isSelf ? "This Computer" : "Connect";
    const disabled = !device.online || isSelf;
    const card = document.createElement("article");
    card.className = `computer-card ${device.online ? "" : "is-offline"} ${isSelf ? "is-self" : ""}`;
    card.innerHTML = `
      <div class="computer-illustration">${computerArt()}</div>
      <h2 class="computer-name">${escapeHtml(device.name || "Gaming PC")}</h2>
      <p class="computer-meta">${isSelf ? "Your Computer" : "Your Computer"}</p>
      <div class="computer-spec">${escapeHtml(device.gpu || "Hardware encoder")} | ${device.online ? "Online" : "Offline"} | ${escapeHtml(device.quality?.label || "1080p")}</div>
      <button class="connect-card-button" ${disabled ? "disabled" : ""} type="button">${buttonText}</button>
    `;
    card.querySelector("button").addEventListener("click", () => connectToDevice(device.id));
    deviceList.appendChild(card);
  }
}

function computerArt() {
  return `
    <span class="pixel-computer">
      <span class="pixel-antenna"></span>
      <span class="gem-logo pixel-screen-logo"></span>
    </span>
  `;
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

async function captureScreen() {
  if (!navigator.mediaDevices?.getDisplayMedia) {
    $("#captureStatus").textContent = "Screen capture is not supported in this runtime";
    return null;
  }
  const quality = appState.hostDevice?.quality || readQuality();
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
  $("#captureStatus").textContent = `${quality.width}x${quality.height} @ ${quality.fps} FPS`;
  return appState.localStream;
}

function stopLocalStream() {
  if (!appState.localStream) return;
  appState.localStream.getTracks().forEach((track) => track.stop());
  appState.localStream = null;
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

  try {
    $(".stream-dock").classList.add("is-visible");
    streamStatus.textContent = "Connecting";
    $("#selectedDeviceLabel").textContent = `${device.name} selected`;
    const data = await api("/api/connect/request", {
      method: "POST",
      body: {
        sessionId: appState.sessionId,
        deviceId: device.id,
        quality: readClientQuality()
      }
    });
    appState.clientRoom = data.room;
    
    const emptyStrong = document.querySelector("#emptyStream strong");
    const emptySpan = document.querySelector("#emptyStream span");
    if (emptyStrong) emptyStrong.textContent = "Đang kết nối...";
    if (emptySpan) emptySpan.textContent = `Đang chờ tín hiệu từ ${device.name}...`;

    
    // Fullscreen is moved to ontrack per user request
  } catch (error) {
    connectError.textContent = error.message;
    streamStatus.textContent = "Idle";
  }
}

async function startHostPeer(room) {
  if (appState.hostPeers.has(room.id)) return;
  if (!appState.hostDevice) {
    await startHost({ capture: false });
  }
  if (!appState.localStream) {
    await captureScreen();
  }
  const pc = createPeerConnection(room, "host");
  appState.hostPeers.set(room.id, pc);
  const inputChannel = pc.createDataChannel("input", {
    ordered: false,
    maxRetransmits: 0
  });
  inputChannel.onmessage = (event) => handleRemoteInput(event.data);

  const videoTrack = appState.localStream.getVideoTracks()[0];
  if (videoTrack) {
    const transceiver = pc.addTransceiver(videoTrack, { direction: "sendonly", streams: [appState.localStream] });
    setCodecPreference(transceiver, room.quality.preferCodec || $("#codecPreference").value);
    await tuneSender(transceiver.sender, room.quality);
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
  $(".stream-dock").classList.add("is-visible");
  appState.clientRoom = room;
  appState.clientPeer = createPeerConnection(room, "client");
  appState.clientPeer.ontrack = async (event) => {
    remoteVideo.srcObject = event.streams[0];
    emptyStream.classList.add("is-hidden");
    streamStatus.textContent = "Streaming";
    $("#clientRoleBadge").classList.remove("is-hidden");
    
    // Request fullscreen ONLY when stream arrives
    try {
      const vs = document.getElementById("videoShell");
      if (vs.requestFullscreen) {
        await vs.requestFullscreen();
      } else if (vs.webkitRequestFullscreen) {
        await vs.webkitRequestFullscreen();
      }
    } catch (e) {
      console.warn("Fullscreen error:", e);
    }
    
    monitorClientStats(appState.clientPeer);
  };
  appState.clientPeer.ondatachannel = (event) => {
    appState.inputChannel = event.channel;
    appState.inputChannel.onopen = () => {
      streamStatus.textContent = "Streaming | input active";
      videoShell.focus();
    };
    appState.inputChannel.onclose = () => {
      streamStatus.textContent = "Streaming | input closed";
    };
  };
}

function createPeerConnection(room, role) {
  const pc = new RTCPeerConnection({
    bundlePolicy: "max-bundle",
    rtcpMuxPolicy: "require",
    iceCandidatePoolSize: 4,
    iceServers: [
      { urls: "stun:stun.l.google.com:19302" }
    ]
  });
  pc.onicecandidate = (event) => {
    if (event.candidate) sendSignal(room.id, { type: "ice", candidate: event.candidate }).catch(() => {});
  };
  pc.onconnectionstatechange = () => {
    const label = role === "host" ? $("#captureStatus") : streamStatus;
    label.textContent = `${role === "host" ? "Host" : "Client"} ${pc.connectionState}`;
  };
  pc.oniceconnectionstatechange = () => {
    if (role === "client" && ["failed", "disconnected"].includes(pc.iceConnectionState)) {
      streamStatus.textContent = `Connection ${pc.iceConnectionState}`;
      connectError.textContent = "Không thể kết nối tới máy chủ! Vui lòng thử lại.";
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
    const answer = await pc.createAnswer();
    await pc.setLocalDescription(answer);
    await sendSignal(data.roomId, { type: "answer", description: pc.localDescription });
  }
  if (message.type === "answer") {
    await pc.setRemoteDescription(message.description);
  }
  if (message.type === "ice" && message.candidate) {
    try {
      await pc.addIceCandidate(message.candidate);
    } catch {
      // ICE candidates can arrive while descriptions are still settling.
    }
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

async function tuneSender(sender, quality) {
  const params = sender.getParameters();
  params.degradationPreference = "maintain-framerate";
  params.encodings = [{
    maxBitrate: quality.bitrateMbps * 1000 * 1000,
    maxFramerate: quality.fps,
    priority: "high",
    networkPriority: "high"
  }];
  try {
    await sender.setParameters(params);
  } catch {
    // Browser support for sender knobs varies.
  }
}

function setCodecPreference(transceiver, preferred) {
  if (!RTCRtpSender.getCapabilities || !transceiver.setCodecPreferences) return;
  const caps = RTCRtpSender.getCapabilities("video");
  if (!caps?.codecs?.length) return;
  const target = String(preferred || "H264").toLowerCase();
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
  const rect = videoShell.getBoundingClientRect();
  const x = clamp01((event.clientX - rect.left) / rect.width);
  const y = clamp01((event.clientY - rect.top) / rect.height);
  return { type, x, y, button: event.button, buttons: event.buttons };
}

function sendInputEvent(payload) {
  if (!appState.inputChannel || appState.inputChannel.readyState !== "open") return;
  appState.inputChannel.send(JSON.stringify({ ...payload, at: performance.now() }));
}

function handleRemoteInput(raw) {
  try {
    const payload = JSON.parse(raw);
    $("#captureStatus").textContent = `Input ${payload.type}`;
  } catch {
    $("#captureStatus").textContent = "Input event";
  }
}

function monitorClientStats(pc) {
  const tick = async () => {
    if (pc !== appState.clientPeer || pc.connectionState === "closed") return;
    const stats = await pc.getStats();
    let fps = "--";
    let bitrate = "--";
    let rtt = "--";
    let lost = 0;
    stats.forEach((report) => {
      if (report.type === "inbound-rtp" && report.kind === "video") {
        fps = Math.round(report.framesPerSecond || 0);
        lost = report.packetsLost || 0;
        if (appState.stats.clientBytes && appState.stats.clientAt) {
          const bytes = Number(report.bytesReceived || 0) - appState.stats.clientBytes;
          const seconds = (report.timestamp - appState.stats.clientAt) / 1000;
          bitrate = seconds > 0 ? ((bytes * 8) / seconds / 1000000).toFixed(1) : "--";
        }
        appState.stats.clientBytes = Number(report.bytesReceived || 0);
        appState.stats.clientAt = report.timestamp;
      }
      if (report.type === "candidate-pair" && report.state === "succeeded" && report.currentRoundTripTime) {
        rtt = Math.round(report.currentRoundTripTime * 1000);
      }
    });
    clientStats.textContent = `FPS ${fps} | ${bitrate} Mbps | RTT ${rtt} ms | Lost ${lost}`;
    setTimeout(tick, 1000);
  };
  tick();
}

function monitorHostStats(pc) {
  const tick = async () => {
    if (pc.connectionState === "closed") return;
    const stats = await pc.getStats();
    let fps = "--";
    let bitrate = "--";
    stats.forEach((report) => {
      if (report.type === "outbound-rtp" && report.kind === "video") {
        fps = Math.round(report.framesPerSecond || report.framesEncoded || 0);
        if (appState.stats.hostBytes && appState.stats.hostAt) {
          const bytes = Number(report.bytesSent || 0) - appState.stats.hostBytes;
          const seconds = (report.timestamp - appState.stats.hostAt) / 1000;
          bitrate = seconds > 0 ? ((bytes * 8) / seconds / 1000000).toFixed(1) : "--";
        }
        appState.stats.hostBytes = Number(report.bytesSent || 0);
        appState.stats.hostAt = report.timestamp;
      }
    });
    hostStats.textContent = `Outbound FPS ${fps} | ${bitrate} Mbps`;
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
  if (appState.clientPeer) appState.clientPeer.close();
  appState.clientPeer = null;
  appState.clientRoom = null;
  appState.inputChannel = null;
  remoteVideo.srcObject = null;
  emptyStream.classList.remove("is-hidden");
  $("#clientRoleBadge").classList.add("is-hidden");
  
  const emptyStrong = document.querySelector("#emptyStream strong");
  const emptySpan = document.querySelector("#emptyStream span");
  if (emptyStrong) emptyStrong.textContent = "No active stream";
  if (emptySpan) emptySpan.textContent = "Choose an online computer.";
  
  streamStatus.textContent = "Idle";
  
  // Exit fullscreen
  try {
    if (document.fullscreenElement) {
      await document.exitFullscreen();
    } else if (document.webkitFullscreenElement) {
      await document.webkitExitFullscreen();
    }
  } catch (e) {}
}

function readQuality() {
  return {
    preset: "1080p",
    width: 1920,
    height: 1080,
    fps: Number($("#hostFps").value || 60),
    bitrateMbps: Number($("#hostBitrate").value || 35),
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
    bitrateMbps: Number($("#clientBitrate").value || 35),
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
}

function applyServerUrl() {
  const nextBase = normalizeServerUrl($("#serverUrlInput").value);
  localStorage.setItem("gr_server_url", nextBase);
  appState.apiBase = nextBase;
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
}

async function api(path, options = {}) {
  const headers = { ...(options.headers || {}) };
  if (appState.token) headers.Authorization = `Bearer ${appState.token}`;
  const init = { ...options, headers };
  if (options.body && typeof options.body !== "string") {
    init.body = JSON.stringify(options.body);
    headers["Content-Type"] = "application/json";
  }
  const response = await fetch(apiUrl(path), init);
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || "Request failed.");
  return data;
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
