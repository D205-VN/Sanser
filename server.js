const crypto = require("crypto");
const fs = require("fs");
const http = require("http");
const path = require("path");
const { URL } = require("url");

const PORT = Number(process.env.PORT || 5174);
const HOST = process.env.HOST || "127.0.0.1";
const PUBLIC_DIR = path.join(__dirname, "public");
const OFFLINE_AFTER_MS = 25000;

let db = null;

function getDataDir() {
  return process.env.GAME_REMOTE_DATA_DIR || path.join(__dirname, "data");
}

function getDataFile() {
  return path.join(getDataDir(), "store.json");
}

function ensureDbLoaded() {
  if (db === null) {
    db = loadStore();
  }
  return db;
}

const mimeTypes = {
  ".html": "text/html; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".json": "application/json; charset=utf-8",
  ".svg": "image/svg+xml; charset=utf-8",
  ".png": "image/png",
  ".ico": "image/x-icon"
};

const state = {
  rooms: new Map(),
  clientsByUser: new Map()
};

function loadStore() {
  try {
    const filename = getDataFile();
    if (!fs.existsSync(filename)) {
      return { users: [], devices: [], tokens: [] };
    }
    const parsed = JSON.parse(fs.readFileSync(filename, "utf8"));
    return {
      users: Array.isArray(parsed.users) ? parsed.users : [],
      devices: Array.isArray(parsed.devices) ? parsed.devices : [],
      tokens: Array.isArray(parsed.tokens) ? parsed.tokens : []
    };
  } catch (error) {
    console.error("Could not load data/store.json:", error.message);
    return { users: [], devices: [], tokens: [] };
  }
}

function saveStore() {
  const filename = getDataFile();
  fs.mkdirSync(path.dirname(filename), { recursive: true });
  fs.writeFileSync(filename, JSON.stringify(db, null, 2));
}

function id(prefix) {
  return `${prefix}_${crypto.randomBytes(10).toString("hex")}`;
}

function hashPassword(password, salt = crypto.randomBytes(16).toString("hex")) {
  const hash = crypto.scryptSync(password, salt, 64).toString("hex");
  return `${salt}:${hash}`;
}

function verifyPassword(password, saved) {
  const [salt, hash] = String(saved || "").split(":");
  if (!salt || !hash) return false;
  const candidate = crypto.scryptSync(password, salt, 64);
  const actual = Buffer.from(hash, "hex");
  return actual.length === candidate.length && crypto.timingSafeEqual(actual, candidate);
}

function sendJson(res, status, payload) {
  const body = JSON.stringify(payload);
  res.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Content-Length": Buffer.byteLength(body),
    "Cache-Control": "no-store",
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Headers": "Content-Type, Authorization",
    "Access-Control-Allow-Methods": "GET, POST, OPTIONS"
  });
  res.end(body);
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    let raw = "";
    req.on("data", (chunk) => {
      raw += chunk;
      if (raw.length > 1024 * 1024) {
        reject(new Error("Request body is too large."));
        req.destroy();
      }
    });
    req.on("end", () => {
      if (!raw.trim()) return resolve({});
      try {
        resolve(JSON.parse(raw));
      } catch (error) {
        reject(new Error("Invalid JSON body."));
      }
    });
    req.on("error", reject);
  });
}

function getBearer(req) {
  const header = req.headers.authorization || "";
  if (header.toLowerCase().startsWith("bearer ")) return header.slice(7).trim();
  return "";
}

function getUserFromToken(token) {
  const row = db.tokens.find((item) => item.token === token);
  if (!row) return null;
  return db.users.find((user) => user.id === row.userId) || null;
}

function publicUser(user) {
  return {
    id: user.id,
    name: user.name,
    email: user.email
  };
}

function cleanOfflineDevices() {
  const now = Date.now();
  let changed = false;
  for (const device of db.devices) {
    if (device.online && now - Number(device.lastSeenAt || 0) > OFFLINE_AFTER_MS) {
      device.online = false;
      changed = true;
    }
  }
  if (changed) saveStore();
}

function publicDevice(device) {
  return {
    id: device.id,
    sessionId: device.sessionId,
    name: device.name,
    gpu: device.gpu,
    platform: device.platform,
    online: Boolean(device.online),
    status: device.status || "ready",
    quality: device.quality,
    autoAccept: Boolean(device.autoAccept),
    lastSeenAt: device.lastSeenAt
  };
}

function sendEvent(userId, event, payload) {
  const clients = state.clientsByUser.get(userId);
  if (!clients) return;
  const data = JSON.stringify(payload);
  for (const res of clients) {
    res.write(`event: ${event}\n`);
    res.write(`data: ${data}\n\n`);
  }
}

function broadcastDevices(userId) {
  cleanOfflineDevices();
  const devices = db.devices.filter((device) => device.userId === userId).map(publicDevice);
  sendEvent(userId, "devices", { devices });
}

function issueToken(userId) {
  const token = crypto.randomBytes(32).toString("hex");
  db.tokens.push({ token, userId, createdAt: Date.now() });
  saveStore();
  return token;
}

function requireAuth(req, res, url) {
  const token = getBearer(req) || url.searchParams.get("token");
  const user = getUserFromToken(token);
  if (!user) {
    sendJson(res, 401, { error: "Not authenticated." });
    return null;
  }
  return { token, user };
}

async function handleApi(req, res, url) {
  if (req.method === "OPTIONS") {
    res.writeHead(204, {
      "Access-Control-Allow-Origin": "*",
      "Access-Control-Allow-Headers": "Content-Type, Authorization",
      "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
      "Access-Control-Max-Age": "86400"
    });
    res.end();
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/health") {
    sendJson(res, 200, { ok: true, now: Date.now() });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/register") {
    const body = await readBody(req);
    const email = String(body.email || "").trim().toLowerCase();
    const password = String(body.password || "");
    const name = String(body.name || "").trim();
    if (!email || password.length < 6 || !name) {
      sendJson(res, 400, { error: "Name, email, and a 6+ character password are required." });
      return;
    }
    if (db.users.some((user) => user.email === email)) {
      sendJson(res, 409, { error: "Email already exists." });
      return;
    }
    const user = { id: id("usr"), name, email, passwordHash: hashPassword(password), createdAt: Date.now() };
    db.users.push(user);
    const token = issueToken(user.id);
    sendJson(res, 201, { token, user: publicUser(user) });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/login") {
    const body = await readBody(req);
    const email = String(body.email || "").trim().toLowerCase();
    const password = String(body.password || "");
    const user = db.users.find((item) => item.email === email);
    if (!user || !verifyPassword(password, user.passwordHash)) {
      sendJson(res, 401, { error: "Invalid email or password." });
      return;
    }
    const token = issueToken(user.id);
    sendJson(res, 200, { token, user: publicUser(user) });
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/events") {
    const token = url.searchParams.get("token") || "";
    const user = getUserFromToken(token);
    if (!user) {
      res.writeHead(401);
      res.end("Not authenticated.");
      return;
    }

    res.writeHead(200, {
      "Content-Type": "text/event-stream; charset=utf-8",
      "Cache-Control": "no-cache, no-transform",
      Connection: "keep-alive",
      "X-Accel-Buffering": "no",
      "Access-Control-Allow-Origin": "*"
    });

    let clients = state.clientsByUser.get(user.id);
    if (!clients) {
      clients = new Set();
      state.clientsByUser.set(user.id, clients);
    }
    clients.add(res);
    res.write(`event: ready\ndata: ${JSON.stringify({ user: publicUser(user), now: Date.now() })}\n\n`);
    broadcastDevices(user.id);

    const keepAlive = setInterval(() => {
      res.write(`event: ping\ndata: ${JSON.stringify({ now: Date.now() })}\n\n`);
    }, 15000);

    req.on("close", () => {
      clearInterval(keepAlive);
      clients.delete(res);
      if (clients.size === 0) state.clientsByUser.delete(user.id);
    });
    return;
  }

  const auth = requireAuth(req, res, url);
  if (!auth) return;
  const { user, token } = auth;

  if (req.method === "GET" && url.pathname === "/api/me") {
    sendJson(res, 200, { user: publicUser(user), token });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/logout") {
    db.tokens = db.tokens.filter((item) => item.token !== token);
    saveStore();
    sendJson(res, 200, { ok: true });
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/devices") {
    cleanOfflineDevices();
    sendJson(res, 200, {
      devices: db.devices.filter((device) => device.userId === user.id).map(publicDevice)
    });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/host/online") {
    const body = await readBody(req);
    const sessionId = String(body.sessionId || "");
    const deviceId = String(body.deviceId || id("dev"));
    const now = Date.now();
    let device = db.devices.find((item) => item.id === deviceId && item.userId === user.id);
    if (!device) {
      device = { id: deviceId, userId: user.id, createdAt: now };
      db.devices.push(device);
    }
    device.sessionId = sessionId;
    device.name = String(body.name || "Gaming PC").trim().slice(0, 80);
    device.gpu = String(body.gpu || "Unknown GPU").trim().slice(0, 80);
    device.platform = String(body.platform || req.headers["user-agent"] || "Unknown").slice(0, 160);
    device.quality = normalizeQuality(body.quality);
    device.autoAccept = Boolean(body.autoAccept);
    device.online = true;
    device.status = "ready";
    device.lastSeenAt = now;
    saveStore();
    sendJson(res, 200, { device: publicDevice(device) });
    broadcastDevices(user.id);
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/host/heartbeat") {
    const body = await readBody(req);
    const device = db.devices.find((item) => item.id === body.deviceId && item.userId === user.id);
    if (!device) {
      sendJson(res, 404, { error: "Device not found." });
      return;
    }
    device.online = true;
    device.lastSeenAt = Date.now();
    if (body.status) device.status = String(body.status).slice(0, 30);
    saveStore();
    sendJson(res, 200, { device: publicDevice(device) });
    broadcastDevices(user.id);
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/host/offline") {
    const body = await readBody(req);
    const device = db.devices.find((item) => item.id === body.deviceId && item.userId === user.id);
    if (device) {
      device.online = false;
      device.status = "offline";
      device.lastSeenAt = Date.now();
      saveStore();
      broadcastDevices(user.id);
    }
    sendJson(res, 200, { ok: true });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/connect/request") {
    const body = await readBody(req);
    const device = db.devices.find((item) => item.id === body.deviceId && item.userId === user.id);
    if (!device || !device.online) {
      sendJson(res, 404, { error: "Host is not online." });
      return;
    }
    const room = {
      id: id("room"),
      userId: user.id,
      hostDeviceId: device.id,
      hostSessionId: device.sessionId,
      clientSessionId: String(body.sessionId || ""),
      clientName: user.name,
      quality: normalizeQuality(body.quality),
      status: "accepted",
      createdAt: Date.now(),
      updatedAt: Date.now()
    };
    state.rooms.set(room.id, room);
    sendJson(res, 200, { room });
    sendEvent(user.id, "connect-request", { room, targetSessionId: room.hostSessionId });
    sendEvent(user.id, "connect-accepted", { room, targetSessionId: room.hostSessionId });
    sendEvent(user.id, "connect-accepted", { room, targetSessionId: room.clientSessionId });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/connect/respond") {
    const body = await readBody(req);
    const room = state.rooms.get(String(body.roomId || ""));
    if (!room || room.userId !== user.id) {
      sendJson(res, 404, { error: "Room not found." });
      return;
    }
    if (String(body.sessionId || "") !== room.hostSessionId) {
      sendJson(res, 403, { error: "Only the host can respond." });
      return;
    }
    room.status = body.accepted ? "accepted" : "rejected";
    room.updatedAt = Date.now();
    const event = body.accepted ? "connect-accepted" : "connect-rejected";
    sendJson(res, 200, { room });
    sendEvent(user.id, event, { room, targetSessionId: room.clientSessionId });
    if (body.accepted) {
      sendEvent(user.id, event, { room, targetSessionId: room.hostSessionId });
    }
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/signaling") {
    const body = await readBody(req);
    const room = state.rooms.get(String(body.roomId || ""));
    if (!room || room.userId !== user.id || room.status !== "accepted") {
      sendJson(res, 404, { error: "Accepted room not found." });
      return;
    }
    const fromSessionId = String(body.sessionId || "");
    const targetSessionId = fromSessionId === room.hostSessionId ? room.clientSessionId : room.hostSessionId;
    if (fromSessionId !== room.hostSessionId && fromSessionId !== room.clientSessionId) {
      sendJson(res, 403, { error: "Session does not belong to this room." });
      return;
    }
    sendEvent(user.id, "signal", {
      roomId: room.id,
      fromSessionId,
      targetSessionId,
      message: body.message || {}
    });
    sendJson(res, 200, { ok: true });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/connect/close") {
    const body = await readBody(req);
    const room = state.rooms.get(String(body.roomId || ""));
    if (room && room.userId === user.id) {
      room.status = "closed";
      room.updatedAt = Date.now();
      sendEvent(user.id, "connect-closed", { room, targetSessionId: room.hostSessionId });
      sendEvent(user.id, "connect-closed", { room, targetSessionId: room.clientSessionId });
    }
    sendJson(res, 200, { ok: true });
    return;
  }

  sendJson(res, 404, { error: "API route not found." });
}

function normalizeQuality(input = {}) {
  const presets = {
    "720p": { label: "720p", width: 1280, height: 720, fps: 60, bitrateMbps: 18 },
    "1080p": { label: "1080p", width: 1920, height: 1080, fps: 60, bitrateMbps: 35 },
    "1440p": { label: "1440p", width: 2560, height: 1440, fps: 60, bitrateMbps: 55 },
    "low-latency": { label: "Low Latency", width: 1600, height: 900, fps: 60, bitrateMbps: 24 }
  };
  const base = presets[input.preset] || presets["1080p"];
  return {
    label: String(input.label || base.label).slice(0, 30),
    width: clamp(Number(input.width || base.width), 640, 3840),
    height: clamp(Number(input.height || base.height), 360, 2160),
    fps: clamp(Number(input.fps || base.fps), 30, 120),
    bitrateMbps: clamp(Number(input.bitrateMbps || base.bitrateMbps), 4, 120),
    preferCodec: String(input.preferCodec || "H264").toUpperCase()
  };
}

function clamp(value, min, max) {
  if (!Number.isFinite(value)) return min;
  return Math.max(min, Math.min(max, Math.round(value)));
}

function serveStatic(req, res, url) {
  const safePath = url.pathname === "/" ? "/index.html" : decodeURIComponent(url.pathname);
  const filePath = path.normalize(path.join(PUBLIC_DIR, safePath));
  if (!filePath.startsWith(PUBLIC_DIR)) {
    res.writeHead(403);
    res.end("Forbidden");
    return;
  }
  fs.readFile(filePath, (error, data) => {
    if (error) {
      res.writeHead(404);
      res.end("Not found");
      return;
    }
    const type = mimeTypes[path.extname(filePath).toLowerCase()] || "application/octet-stream";
    res.writeHead(200, {
      "Content-Type": type,
      "Cache-Control": "no-store"
    });
    res.end(data);
  });
}

function createAppServer() {
  return http.createServer(async (req, res) => {
    const url = new URL(req.url, `http://${req.headers.host || "localhost"}`);
    try {
      if (url.pathname.startsWith("/api/")) {
        await handleApi(req, res, url);
        return;
      }
      serveStatic(req, res, url);
    } catch (error) {
      console.error(error);
      if (!res.headersSent) sendJson(res, 500, { error: error.message || "Server error." });
    }
  });
}

function startServer(options = {}) {
  const host = options.host || HOST;
  const port = Number.isFinite(Number(options.port)) ? Number(options.port) : PORT;
  ensureDbLoaded();
  const server = createAppServer();

  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, host, () => {
      server.off("error", reject);
      const address = server.address();
      const actualPort = typeof address === "object" && address ? address.port : port;
      console.log(`GameRemote running at http://${host}:${actualPort}`);
      if (host === "127.0.0.1") {
        console.log("Use HOST=0.0.0.0 npm run dev to test from another computer on the same LAN.");
      }
      resolve({ server, host, port: actualPort });
    });
  });
}

if (require.main === module) {
  startServer().catch((error) => {
    console.error(error);
    process.exit(1);
  });
}

module.exports = {
  createAppServer,
  startServer
};
