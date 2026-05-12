require('dotenv').config();
const crypto = require("crypto");
const fs = require("fs");
const http = require("http");
const path = require("path");
const { URL } = require("url");
const { Pool } = require("pg");

const PORT = Number(process.env.PORT || 5174);
const HOST = process.env.HOST || "127.0.0.1";
const PUBLIC_DIR = path.join(__dirname, "public");
const OFFLINE_AFTER_MS = 25000;

const pool = new Pool({
  connectionString: process.env.DATABASE_URL
});

async function initDb() {
  const client = await pool.connect();
  try {
    await client.query(`
      CREATE TABLE IF NOT EXISTS users (
        id VARCHAR(255) PRIMARY KEY,
        name VARCHAR(255) NOT NULL,
        email VARCHAR(255) UNIQUE NOT NULL,
        password_hash VARCHAR(255) NOT NULL,
        created_at BIGINT NOT NULL
      );
    `);
    await client.query(`
      CREATE TABLE IF NOT EXISTS tokens (
        token VARCHAR(255) PRIMARY KEY,
        user_id VARCHAR(255) REFERENCES users(id) ON DELETE CASCADE,
        created_at BIGINT NOT NULL
      );
    `);
    await client.query(`
      CREATE TABLE IF NOT EXISTS devices (
        id VARCHAR(255) PRIMARY KEY,
        user_id VARCHAR(255) REFERENCES users(id) ON DELETE CASCADE,
        session_id VARCHAR(255),
        name VARCHAR(255),
        gpu VARCHAR(255),
        platform VARCHAR(255),
        quality JSONB,
        auto_accept BOOLEAN DEFAULT FALSE,
        online BOOLEAN DEFAULT FALSE,
        status VARCHAR(255) DEFAULT 'ready',
        last_seen_at BIGINT,
        created_at BIGINT NOT NULL
      );
    `);
  } finally {
    client.release();
  }
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

async function getUserFromToken(token) {
  const res = await pool.query('SELECT u.* FROM tokens t JOIN users u ON t.user_id = u.id WHERE t.token = $1', [token]);
  if (res.rows.length === 0) return null;
  return res.rows[0];
}

function publicUser(user) {
  return {
    id: user.id,
    name: user.name,
    email: user.email
  };
}

async function cleanOfflineDevices() {
  const cutoff = Date.now() - OFFLINE_AFTER_MS;
  await pool.query('UPDATE devices SET online = false WHERE online = true AND last_seen_at <= $1', [cutoff]);
}

function publicDevice(device) {
  return {
    id: device.id,
    sessionId: device.session_id,
    name: device.name,
    gpu: device.gpu,
    platform: device.platform,
    online: Boolean(device.online),
    status: device.status || "ready",
    quality: device.quality,
    autoAccept: Boolean(device.auto_accept),
    lastSeenAt: Number(device.last_seen_at)
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

async function broadcastDevices(userId) {
  await cleanOfflineDevices();
  const res = await pool.query('SELECT * FROM devices WHERE user_id = $1', [userId]);
  const devices = res.rows.map(publicDevice);
  sendEvent(userId, "devices", { devices });
}

async function issueToken(userId) {
  const token = crypto.randomBytes(32).toString("hex");
  await pool.query('INSERT INTO tokens (token, user_id, created_at) VALUES ($1, $2, $3)', [token, userId, Date.now()]);
  return token;
}

async function requireAuth(req, res, url) {
  const token = getBearer(req) || url.searchParams.get("token");
  const user = await getUserFromToken(token);
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
    const existing = await pool.query('SELECT id FROM users WHERE email = $1', [email]);
    if (existing.rows.length > 0) {
      sendJson(res, 409, { error: "Email already exists." });
      return;
    }
    const userId = id("usr");
    const user = { id: userId, name, email, passwordHash: hashPassword(password), createdAt: Date.now() };
    await pool.query('INSERT INTO users (id, name, email, password_hash, created_at) VALUES ($1, $2, $3, $4, $5)', 
      [user.id, user.name, user.email, user.passwordHash, user.createdAt]);
    const token = await issueToken(user.id);
    sendJson(res, 201, { token, user: publicUser(user) });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/login") {
    const body = await readBody(req);
    const email = String(body.email || "").trim().toLowerCase();
    const password = String(body.password || "");
    const existing = await pool.query('SELECT * FROM users WHERE email = $1', [email]);
    if (existing.rows.length === 0) {
      sendJson(res, 401, { error: "Invalid email or password." });
      return;
    }
    const user = existing.rows[0];
    if (!verifyPassword(password, user.password_hash)) {
      sendJson(res, 401, { error: "Invalid email or password." });
      return;
    }
    const token = await issueToken(user.id);
    sendJson(res, 200, { token, user: publicUser(user) });
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/events") {
    const token = url.searchParams.get("token") || "";
    const user = await getUserFromToken(token);
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
    await broadcastDevices(user.id);

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

  const auth = await requireAuth(req, res, url);
  if (!auth) return;
  const { user, token } = auth;

  if (req.method === "GET" && url.pathname === "/api/me") {
    sendJson(res, 200, { user: publicUser(user), token });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/logout") {
    await pool.query('DELETE FROM tokens WHERE token = $1', [token]);
    sendJson(res, 200, { ok: true });
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/devices") {
    await cleanOfflineDevices();
    const resDevices = await pool.query('SELECT * FROM devices WHERE user_id = $1', [user.id]);
    sendJson(res, 200, {
      devices: resDevices.rows.map(publicDevice)
    });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/host/online") {
    const body = await readBody(req);
    const sessionId = String(body.sessionId || "");
    const deviceId = String(body.deviceId || id("dev"));
    const now = Date.now();
    
    const existing = await pool.query('SELECT * FROM devices WHERE id = $1 AND user_id = $2', [deviceId, user.id]);
    const name = String(body.name || "Gaming PC").trim().slice(0, 80);
    const gpu = String(body.gpu || "Unknown GPU").trim().slice(0, 80);
    const platform = String(body.platform || req.headers["user-agent"] || "Unknown").slice(0, 160);
    const quality = normalizeQuality(body.quality);
    const autoAccept = Boolean(body.autoAccept);
    
    if (existing.rows.length === 0) {
      await pool.query(`
        INSERT INTO devices (id, user_id, session_id, name, gpu, platform, quality, auto_accept, online, status, last_seen_at, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, true, 'ready', $9, $9)
      `, [deviceId, user.id, sessionId, name, gpu, platform, quality, autoAccept, now]);
    } else {
      await pool.query(`
        UPDATE devices 
        SET session_id = $1, name = $2, gpu = $3, platform = $4, quality = $5, auto_accept = $6, online = true, status = 'ready', last_seen_at = $7
        WHERE id = $8 AND user_id = $9
      `, [sessionId, name, gpu, platform, quality, autoAccept, now, deviceId, user.id]);
    }
    
    const updated = await pool.query('SELECT * FROM devices WHERE id = $1', [deviceId]);
    sendJson(res, 200, { device: publicDevice(updated.rows[0]) });
    await broadcastDevices(user.id);
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/host/heartbeat") {
    const body = await readBody(req);
    const existing = await pool.query('SELECT * FROM devices WHERE id = $1 AND user_id = $2', [body.deviceId, user.id]);
    if (existing.rows.length === 0) {
      sendJson(res, 404, { error: "Device not found." });
      return;
    }
    const status = body.status ? String(body.status).slice(0, 30) : existing.rows[0].status;
    await pool.query('UPDATE devices SET online = true, last_seen_at = $1, status = $2 WHERE id = $3 AND user_id = $4', 
      [Date.now(), status, body.deviceId, user.id]);
      
    const updated = await pool.query('SELECT * FROM devices WHERE id = $1', [body.deviceId]);
    sendJson(res, 200, { device: publicDevice(updated.rows[0]) });
    await broadcastDevices(user.id);
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/host/offline") {
    const body = await readBody(req);
    const existing = await pool.query('SELECT * FROM devices WHERE id = $1 AND user_id = $2', [body.deviceId, user.id]);
    if (existing.rows.length > 0) {
      await pool.query('UPDATE devices SET online = false, status = $1, last_seen_at = $2 WHERE id = $3 AND user_id = $4', 
        ['offline', Date.now(), body.deviceId, user.id]);
      await broadcastDevices(user.id);
    }
    sendJson(res, 200, { ok: true });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/connect/request") {
    const body = await readBody(req);
    const existing = await pool.query('SELECT * FROM devices WHERE id = $1 AND user_id = $2', [body.deviceId, user.id]);
    if (existing.rows.length === 0 || !existing.rows[0].online) {
      sendJson(res, 404, { error: "Host is not online." });
      return;
    }
    const device = existing.rows[0];
    const room = {
      id: id("room"),
      userId: user.id,
      hostDeviceId: device.id,
      hostSessionId: device.session_id,
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

async function startServer(options = {}) {
  const host = options.host || HOST;
  const port = Number.isFinite(Number(options.port)) ? Number(options.port) : PORT;
  
  await initDb();
  
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
