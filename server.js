require("dotenv").config({ quiet: true });
const crypto = require("crypto");
const fs = require("fs");
const http = require("http");
const net = require("net");
const path = require("path");
const { URL } = require("url");
const { Pool } = require("pg");

const PORT = Number(process.env.PORT || 5174);
const HOST = process.env.HOST || "127.0.0.1";
const PUBLIC_DIR = path.join(__dirname, "public");
const OFFLINE_AFTER_MS = 25000;
const OFFLINE_SWEEP_MS = 5000;
const CONNECTION_REQUEST_TTL_MS = 30 * 1000;
const EVENT_RETENTION_MS = 24 * 60 * 60 * 1000;
const ROOM_RETENTION_MS = 24 * 60 * 60 * 1000;
const ACTIVE_ROOM_RETENTION_MS = 7 * 24 * 60 * 60 * 1000;
const TOKEN_RETENTION_MS = 30 * 24 * 60 * 60 * 1000;
const MAINTENANCE_INTERVAL_MS = 60 * 1000;
const MAX_JSON_BODY_BYTES = 1024 * 1024;
const MAX_SNV_HOST_LENGTH = 255;
const AUTH_FAILURE_WINDOW_MS = 10 * 60 * 1000;
const AUTH_BLOCK_MS = 60 * 1000;
const AUTH_FAILURE_LIMIT = 8;
const REGISTRATION_WINDOW_MS = 10 * 60 * 1000;
const REGISTRATION_LIMIT = 5;
const DEFAULT_STUN_URLS = "stun:stun.l.google.com:19302";

function normalizeDatabaseUrl(value) {
  const connectionString = String(value || "");
  if (!connectionString) return connectionString;
  try {
    const parsed = new URL(connectionString);
    const sslMode = parsed.searchParams.get("sslmode");
    if (["prefer", "require", "verify-ca"].includes(sslMode)) {
      parsed.searchParams.set("sslmode", "verify-full");
    }
    return parsed.toString();
  } catch {
    return connectionString;
  }
}

const pool = new Pool({
  connectionString: normalizeDatabaseUrl(process.env.DATABASE_URL)
});
let poolClosed = false;

class HttpError extends Error {
  constructor(status, message) {
    super(message);
    this.name = "HttpError";
    this.status = status;
  }
}

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
        ip VARCHAR(255),
        quality JSONB,
        auto_accept BOOLEAN DEFAULT FALSE,
        online BOOLEAN DEFAULT FALSE,
        status VARCHAR(255) DEFAULT 'ready',
        last_seen_at BIGINT,
        created_at BIGINT NOT NULL
      );
    `);
    // Add ip column if missing (migration for existing databases)
    await client.query(`
      ALTER TABLE devices ADD COLUMN IF NOT EXISTS ip VARCHAR(255);
    `).catch(() => {});
    await client.query(`
      CREATE TABLE IF NOT EXISTS connection_rooms (
        id VARCHAR(255) PRIMARY KEY,
        user_id VARCHAR(255) REFERENCES users(id) ON DELETE CASCADE,
        host_device_id VARCHAR(255),
        host_session_id VARCHAR(255),
        client_session_id VARCHAR(255),
        client_name VARCHAR(255),
        quality JSONB,
        native JSONB,
        status VARCHAR(255) DEFAULT 'accepted',
        created_at BIGINT NOT NULL,
        updated_at BIGINT NOT NULL
      );
    `);
    await client.query(`
      CREATE TABLE IF NOT EXISTS app_events (
        id BIGSERIAL PRIMARY KEY,
        user_id VARCHAR(255) REFERENCES users(id) ON DELETE CASCADE,
        event VARCHAR(255) NOT NULL,
        payload JSONB NOT NULL,
        created_at BIGINT NOT NULL
      );
    `);
    await client.query(`
      CREATE INDEX IF NOT EXISTS app_events_user_id_id_idx ON app_events (user_id, id);
    `);
    await client.query(`
      ALTER TABLE connection_rooms ADD COLUMN IF NOT EXISTS native JSONB;
    `);
    await client.query(`
      CREATE INDEX IF NOT EXISTS devices_online_last_seen_idx ON devices (online, last_seen_at);
    `);
    await client.query(`
      CREATE INDEX IF NOT EXISTS connection_rooms_updated_at_idx ON connection_rooms (updated_at);
    `);
    await client.query(`
      CREATE INDEX IF NOT EXISTS tokens_created_at_idx ON tokens (created_at);
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
  clientsByUser: new Map(),
  authFailures: new Map(),
  registrationAttempts: new Map()
};

function id(prefix) {
  return `${prefix}_${crypto.randomBytes(10).toString("hex")}`;
}

function scrypt(password, salt) {
  return new Promise((resolve, reject) => {
    crypto.scrypt(password, salt, 64, (error, derivedKey) => {
      if (error) reject(error);
      else resolve(derivedKey);
    });
  });
}

async function hashPassword(password, salt = crypto.randomBytes(16).toString("hex")) {
  const hash = (await scrypt(password, salt)).toString("hex");
  return `${salt}:${hash}`;
}

async function verifyPassword(password, saved) {
  const [salt, hash] = String(saved || "").split(":");
  if (!salt || !hash) return false;
  const candidate = await scrypt(password, salt);
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
    "Access-Control-Allow-Methods": "GET, POST, OPTIONS",
    "Referrer-Policy": "no-referrer",
    "X-Content-Type-Options": "nosniff"
  });
  res.end(body);
}

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    let settled = false;
    req.on("data", (chunk) => {
      if (settled) return;
      size += chunk.length;
      if (size > MAX_JSON_BODY_BYTES) {
        settled = true;
        reject(new HttpError(413, "Request body is too large."));
        return;
      }
      chunks.push(chunk);
    });
    req.on("end", () => {
      if (settled) return;
      const raw = Buffer.concat(chunks).toString("utf8");
      if (!raw.trim()) return resolve({});
      try {
        resolve(JSON.parse(raw));
      } catch {
        reject(new HttpError(400, "Invalid JSON body."));
      }
    });
    req.on("error", (error) => {
      if (!settled) reject(error);
    });
  });
}

function getBearer(req) {
  const header = req.headers.authorization || "";
  if (header.toLowerCase().startsWith("bearer ")) return header.slice(7).trim();
  return "";
}

function tokenDigest(token) {
  return `sha256:${crypto.createHash("sha256").update(String(token || "")).digest("hex")}`;
}

async function getUserFromToken(token) {
  if (!token) return null;
  const digest = tokenDigest(token);
  const res = await pool.query(
    'SELECT u.*, t.token AS stored_token FROM tokens t JOIN users u ON t.user_id = u.id WHERE t.token = ANY($1) AND t.created_at > $2 LIMIT 1',
    [[digest, token], Date.now() - TOKEN_RETENTION_MS]
  );
  if (res.rows.length === 0) return null;
  if (res.rows[0].stored_token === token) {
    await pool.query('UPDATE tokens SET token = $1 WHERE token = $2', [digest, token]).catch(() => {});
  }
  return res.rows[0];
}

function publicUser(user) {
  return {
    id: user.id,
    name: user.name,
    email: user.email
  };
}

function isValidEmail(value) {
  const email = String(value || "");
  return email.length <= 254 && /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email);
}

function parseCsv(value) {
  return String(value || "")
    .split(",")
    .map((item) => item.trim())
    .filter(Boolean);
}

function getIceServers() {
  const mode = networkMode();
  if (mode === "tailscale" && process.env.TAILSCALE_USE_STUN !== "1") {
    return [];
  }
  const stunUrls = parseCsv(process.env.STUN_URLS || DEFAULT_STUN_URLS);
  const turnUrls = parseCsv(process.env.TURN_URLS);
  const iceServers = [];
  if (mode !== "relay" && stunUrls.length) iceServers.push({ urls: stunUrls });
  if (mode !== "direct" && turnUrls.length) {
    const turnServer = { urls: turnUrls };
    if (process.env.TURN_USERNAME) turnServer.username = process.env.TURN_USERNAME;
    if (process.env.TURN_CREDENTIAL) turnServer.credential = process.env.TURN_CREDENTIAL;
    iceServers.push(turnServer);
  }
  return iceServers;
}

function hasTurnServer(iceServers = getIceServers()) {
  return iceServers.some((server) => parseCsv(server.urls)
    .some((item) => item.toLowerCase().startsWith("turn:")));
}

function networkMode() {
  const configured = String(process.env.NETWORK_MODE || "default").trim().toLowerCase();
  if (configured === "auto" || !configured) return "default";
  return ["default", "direct", "relay", "tailscale"].includes(configured) ? configured : "default";
}

function iceTransportPolicy() {
  const mode = networkMode();
  if (mode === "relay") return "relay";
  if (mode === "direct" || mode === "tailscale") return "all";
  const configured = String(process.env.ICE_TRANSPORT_POLICY || "").trim().toLowerCase();
  if (configured === "all" || configured === "relay") return configured;
  return "all";
}

async function cleanOfflineDevices() {
  const cutoff = Date.now() - OFFLINE_AFTER_MS;
  const result = await pool.query(
    "UPDATE devices SET online = false, status = 'offline' WHERE online = true AND last_seen_at <= $1 RETURNING user_id",
    [cutoff]
  );
  return [...new Set(result.rows.map((row) => row.user_id).filter(Boolean))];
}

function publicDevice(device) {
  return {
    id: device.id,
    sessionId: device.session_id,
    name: device.name,
    gpu: device.gpu,
    platform: device.platform,
    ip: device.ip || "",
    online: Boolean(device.online),
    status: device.status || "ready",
    quality: device.quality,
    autoAccept: Boolean(device.auto_accept),
    lastSeenAt: Number(device.last_seen_at)
  };
}

function publicRoom(room) {
  return {
    id: room.id,
    userId: room.user_id || room.userId,
    hostDeviceId: room.host_device_id || room.hostDeviceId,
    hostSessionId: room.host_session_id || room.hostSessionId,
    clientSessionId: room.client_session_id || room.clientSessionId,
    clientName: room.client_name || room.clientName,
    quality: normalizeQuality(room.quality),
    native: room.native || null,
    status: room.status || "accepted",
    createdAt: Number(room.created_at || room.createdAt),
    updatedAt: Number(room.updated_at || room.updatedAt)
  };
}

async function getCurrentEventId(userId) {
  const result = await pool.query('SELECT COALESCE(MAX(id), 0) AS id FROM app_events WHERE user_id = $1', [userId]);
  return Number(result.rows[0]?.id || 0);
}

function writeSse(client, eventId, event, payload) {
  if (client.res.destroyed || client.res.writableEnded) return false;
  const numericEventId = Number(eventId || 0);
  if (numericEventId > 0) client.res.write(`id: ${numericEventId}\n`);
  client.res.write(`event: ${event}\n`);
  const writable = client.res.write(`data: ${JSON.stringify(payload)}\n\n`);
  if (numericEventId > 0) {
    client.lastEventId = Math.max(client.lastEventId, numericEventId);
  }
  return writable;
}

async function sendEvent(userId, event, payload) {
  const inserted = await pool.query(
    'INSERT INTO app_events (user_id, event, payload, created_at) VALUES ($1, $2, $3, $4) RETURNING id',
    [userId, event, payload, Date.now()]
  );
  const eventId = Number(inserted.rows[0].id);
  const clients = state.clientsByUser.get(userId);
  if (!clients) return;
  for (const client of clients) {
    if (client.polling) continue;
    try {
      writeSse(client, eventId, event, payload);
    } catch {
      clients.delete(client);
    }
  }
  if (clients.size === 0) state.clientsByUser.delete(userId);
}

async function pollEvents(userId, client) {
  const result = await pool.query(
    'SELECT id, event, payload FROM app_events WHERE user_id = $1 AND id > $2 ORDER BY id ASC LIMIT 100',
    [userId, client.lastEventId]
  );
  for (const row of result.rows) {
    if (Number(row.id) <= client.lastEventId) continue;
    writeSse(client, Number(row.id), row.event, row.payload);
  }
  return result.rows.length;
}

async function replayEvents(userId, client) {
  let count = 0;
  do {
    count = await pollEvents(userId, client);
  } while (count === 100 && !client.res.destroyed && !client.res.writableEnded);
}

async function getRoom(roomId) {
  if (!roomId) return null;
  const cached = state.rooms.get(roomId);
  if (cached) return cached;
  const result = await pool.query('SELECT * FROM connection_rooms WHERE id = $1', [roomId]);
  if (result.rows.length === 0) return null;
  const room = publicRoom(result.rows[0]);
  state.rooms.set(room.id, room);
  return room;
}

async function broadcastDevices(userId) {
  await cleanOfflineDevices();
  await broadcastDeviceSnapshot(userId);
}

async function broadcastDeviceSnapshot(userId) {
  const res = await pool.query('SELECT * FROM devices WHERE user_id = $1', [userId]);
  const devices = res.rows.map(publicDevice);
  await sendEvent(userId, "devices", { devices });
}

async function expireOfflineDevices() {
  const userIds = await cleanOfflineDevices();
  for (const userId of userIds) {
    await broadcastDeviceSnapshot(userId);
  }
}

async function expirePendingRooms() {
  const expired = await pool.query(
    "UPDATE connection_rooms SET status = 'expired', updated_at = $1 WHERE status = 'pending' AND updated_at <= $2 RETURNING *",
    [Date.now(), Date.now() - CONNECTION_REQUEST_TTL_MS]
  );
  for (const row of expired.rows) {
    const room = publicRoom(row);
    state.rooms.set(room.id, room);
    await sendEvent(room.userId, "connect-rejected", {
      room,
      reason: "expired",
      targetSessionId: room.clientSessionId
    });
    await sendEvent(room.userId, "connect-rejected", {
      room,
      reason: "expired",
      targetSessionId: room.hostSessionId
    });
  }
}

async function runMaintenance() {
  const now = Date.now();
  await pool.query('DELETE FROM app_events WHERE created_at <= $1', [now - EVENT_RETENTION_MS]);
  await pool.query('DELETE FROM tokens WHERE created_at <= $1', [now - TOKEN_RETENTION_MS]);
  await pool.query(
    "DELETE FROM connection_rooms WHERE (status IN ('closed', 'rejected', 'expired') AND updated_at <= $1) OR updated_at <= $2",
    [now - ROOM_RETENTION_MS, now - ACTIVE_ROOM_RETENTION_MS]
  );
  for (const [roomId, room] of state.rooms) {
    const updatedAt = Number(room.updatedAt || room.updated_at || 0);
    const terminal = ["closed", "rejected", "expired"].includes(room.status);
    if ((terminal && updatedAt <= now - ROOM_RETENTION_MS) ||
        updatedAt <= now - ACTIVE_ROOM_RETENTION_MS) {
      state.rooms.delete(roomId);
    }
  }
  for (const [key, entry] of state.authFailures) {
    const lastRelevantAt = Math.max(Number(entry.startedAt || 0), Number(entry.blockedUntil || 0));
    if (lastRelevantAt <= now - AUTH_FAILURE_WINDOW_MS) state.authFailures.delete(key);
  }
  for (const [key, entry] of state.registrationAttempts) {
    if (Number(entry.startedAt || 0) <= now - REGISTRATION_WINDOW_MS) {
      state.registrationAttempts.delete(key);
    }
  }
}

function parseLastEventId(value) {
  const eventId = Number.parseInt(String(value || ""), 10);
  return Number.isSafeInteger(eventId) && eventId >= 0 ? eventId : null;
}

function requireIdentifier(value, label) {
  const normalized = String(value || "").trim();
  if (!normalized || normalized.length > 255 || !/^[A-Za-z0-9._~-]+$/.test(normalized)) {
    throw new HttpError(400, `${label} is invalid.`);
  }
  return normalized;
}

function authFailureKeys(req, email) {
  const ip = String(req.socket.remoteAddress || "unknown").replace(/^::ffff:/, "");
  return [`ip:${ip}`, `account:${ip}:${String(email || "").toLowerCase()}`];
}

function assertAuthAttemptAllowed(req, email) {
  const now = Date.now();
  for (const key of authFailureKeys(req, email)) {
    const entry = state.authFailures.get(key);
    if (entry?.blockedUntil > now) {
      throw new HttpError(429, "Too many login attempts. Please wait and try again.");
    }
  }
}

function recordAuthFailure(req, email) {
  const now = Date.now();
  for (const key of authFailureKeys(req, email)) {
    const previous = state.authFailures.get(key);
    const entry = !previous || now - previous.startedAt > AUTH_FAILURE_WINDOW_MS
      ? { count: 0, startedAt: now, blockedUntil: 0 }
      : previous;
    entry.count += 1;
    if (entry.count >= AUTH_FAILURE_LIMIT) entry.blockedUntil = now + AUTH_BLOCK_MS;
    state.authFailures.set(key, entry);
  }
}

function clearAuthFailures(req, email) {
  for (const key of authFailureKeys(req, email)) state.authFailures.delete(key);
}

function consumeRegistrationAttempt(req) {
  const ip = String(req.socket.remoteAddress || "unknown").replace(/^::ffff:/, "");
  const now = Date.now();
  const previous = state.registrationAttempts.get(ip);
  const entry = !previous || now - previous.startedAt > REGISTRATION_WINDOW_MS
    ? { count: 0, startedAt: now }
    : previous;
  if (entry.count >= REGISTRATION_LIMIT) {
    throw new HttpError(429, "Too many registration attempts. Please wait and try again.");
  }
  entry.count += 1;
  state.registrationAttempts.set(ip, entry);
}

function normalizeOptionalIp(value, fallback) {
  const candidate = String(value || fallback || "").trim().replace(/^::ffff:/, "");
  const normalized = candidate === "::1" ? "127.0.0.1" : candidate;
  if (!normalized || normalized.length > MAX_SNV_HOST_LENGTH || net.isIP(normalized) === 0) {
    throw new HttpError(400, "Native client IP is invalid.");
  }
  return normalized;
}

async function issueToken(userId) {
  const token = crypto.randomBytes(32).toString("hex");
  await pool.query(
    'INSERT INTO tokens (token, user_id, created_at) VALUES ($1, $2, $3)',
    [tokenDigest(token), userId, Date.now()]
  );
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
    consumeRegistrationAttempt(req);
    const body = await readBody(req);
    const email = String(body.email || "").trim().toLowerCase();
    const password = String(body.password || "");
    const name = String(body.name || "").trim();
    if (!isValidEmail(email) || password.length < 6 || password.length > 1024 || !name) {
      sendJson(res, 400, { error: "Name, email, and a 6+ character password are required." });
      return;
    }
    const existing = await pool.query('SELECT id FROM users WHERE email = $1', [email]);
    if (existing.rows.length > 0) {
      sendJson(res, 409, { error: "Email already exists." });
      return;
    }
    const userId = id("usr");
    const user = {
      id: userId,
      name: name.slice(0, 255),
      email: email.slice(0, 255),
      passwordHash: await hashPassword(password),
      createdAt: Date.now()
    };
    try {
      await pool.query('INSERT INTO users (id, name, email, password_hash, created_at) VALUES ($1, $2, $3, $4, $5)',
        [user.id, user.name, user.email, user.passwordHash, user.createdAt]);
    } catch (error) {
      if (error?.code === "23505") {
        sendJson(res, 409, { error: "Email already exists." });
        return;
      }
      throw error;
    }
    const token = await issueToken(user.id);
    sendJson(res, 201, { token, user: publicUser(user) });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/login") {
    const body = await readBody(req);
    const email = String(body.email || "").trim().toLowerCase();
    const password = String(body.password || "");
    assertAuthAttemptAllowed(req, email);
    if (!isValidEmail(email) || !password || password.length > 1024) {
      recordAuthFailure(req, email);
      sendJson(res, 401, { error: "Invalid email or password." });
      return;
    }
    const existing = await pool.query('SELECT * FROM users WHERE email = $1', [email]);
    if (existing.rows.length === 0) {
      recordAuthFailure(req, email);
      sendJson(res, 401, { error: "Invalid email or password." });
      return;
    }
    const user = existing.rows[0];
    if (!(await verifyPassword(password, user.password_hash))) {
      recordAuthFailure(req, email);
      sendJson(res, 401, { error: "Invalid email or password." });
      return;
    }
    clearAuthFailures(req, email);
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
      "Access-Control-Allow-Origin": "*",
      "Referrer-Policy": "no-referrer",
      "X-Content-Type-Options": "nosniff"
    });

    let clients = state.clientsByUser.get(user.id);
    if (!clients) {
      clients = new Set();
      state.clientsByUser.set(user.id, clients);
    }
    const currentEventId = await getCurrentEventId(user.id);
    const requestedEventId = parseLastEventId(req.headers["last-event-id"]);
    const client = {
      res,
      lastEventId: requestedEventId === null ? currentEventId : Math.min(requestedEventId, currentEventId),
      polling: false,
      close: null
    };
    let keepAlive = null;
    let eventPoller = null;
    let clientClosed = false;
    const closeClient = (endResponse = false) => {
      if (clientClosed) return;
      clientClosed = true;
      if (keepAlive) clearInterval(keepAlive);
      if (eventPoller) clearInterval(eventPoller);
      clients.delete(client);
      if (clients.size === 0) state.clientsByUser.delete(user.id);
      if (endResponse && !res.destroyed && !res.writableEnded) res.end();
    };
    client.close = () => closeClient(true);
    clients.add(client);
    if (requestedEventId !== null) {
      client.polling = true;
      try {
        await replayEvents(user.id, client);
      } finally {
        client.polling = false;
      }
    }
    writeSse(client, null, "ready", { user: publicUser(user), now: Date.now() });
    await broadcastDevices(user.id);

    keepAlive = setInterval(() => {
      if (!res.destroyed && !res.writableEnded) {
        res.write(`event: ping\ndata: ${JSON.stringify({ now: Date.now() })}\n\n`);
      }
    }, 15000);
    eventPoller = setInterval(async () => {
      if (client.polling) return;
      client.polling = true;
      try {
        await replayEvents(user.id, client);
      } catch (error) {
        console.error("Event poll failed:", error.message);
      } finally {
        client.polling = false;
      }
    }, 1000);

    req.on("close", () => closeClient(false));
    return;
  }

  const auth = await requireAuth(req, res, url);
  if (!auth) return;
  const { user, token } = auth;

  if (req.method === "GET" && url.pathname === "/api/config") {
    const iceServers = getIceServers();
    const hasTurn = hasTurnServer(iceServers);
    const mode = networkMode();
    sendJson(res, 200, {
      iceServers,
      hasTurn,
      networkMode: mode,
      relayReady: mode !== "relay" || hasTurn,
      iceTransportPolicy: iceTransportPolicy()
    });
    return;
  }

  if (req.method === "GET" && url.pathname === "/api/me") {
    sendJson(res, 200, { user: publicUser(user), token });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/logout") {
    await pool.query('DELETE FROM tokens WHERE token = ANY($1)', [[tokenDigest(token), token]]);
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
    const sessionId = requireIdentifier(body.sessionId, "Session ID");
    const deviceId = requireIdentifier(body.deviceId || id("dev"), "Device ID");
    const now = Date.now();
    
    // Capture the host's IP address
    const rawIp = req.socket.remoteAddress || "";
    const hostIp = rawIp.replace(/^::ffff:/, "");
    
    const existing = await pool.query('SELECT * FROM devices WHERE id = $1 AND user_id = $2', [deviceId, user.id]);
    const name = String(body.name || "Gaming PC").trim().slice(0, 80);
    const gpu = String(body.gpu || "Unknown GPU").trim().slice(0, 80);
    const platform = String(body.platform || req.headers["user-agent"] || "Unknown").slice(0, 160);
    const quality = normalizeQuality(body.quality);
    const autoAccept = Boolean(body.autoAccept);
    
    if (existing.rows.length === 0) {
      await pool.query(`
        INSERT INTO devices (id, user_id, session_id, name, gpu, platform, ip, quality, auto_accept, online, status, last_seen_at, created_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, true, 'ready', $10, $10)
      `, [deviceId, user.id, sessionId, name, gpu, platform, hostIp, quality, autoAccept, now]);
    } else {
      await pool.query(`
        UPDATE devices 
        SET session_id = $1, name = $2, gpu = $3, platform = $4, quality = $5, auto_accept = $6, ip = $7, online = true, status = 'ready', last_seen_at = $8
        WHERE id = $9 AND user_id = $10
      `, [sessionId, name, gpu, platform, quality, autoAccept, hostIp, now, deviceId, user.id]);
    }
    
    const updated = await pool.query('SELECT * FROM devices WHERE id = $1', [deviceId]);
    sendJson(res, 200, { device: publicDevice(updated.rows[0]) });
    await broadcastDevices(user.id);
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/host/heartbeat") {
    const body = await readBody(req);
    const deviceId = requireIdentifier(body.deviceId, "Device ID");
    const sessionId = requireIdentifier(body.sessionId, "Session ID");
    const existing = await pool.query(
      'SELECT * FROM devices WHERE id = $1 AND user_id = $2 AND session_id = $3',
      [deviceId, user.id, sessionId]
    );
    if (existing.rows.length === 0) {
      sendJson(res, 404, { error: "Device not found." });
      return;
    }
    const status = body.status ? String(body.status).slice(0, 30) : existing.rows[0].status;
    const shouldBroadcast = !existing.rows[0].online || existing.rows[0].status !== status;
    await pool.query('UPDATE devices SET online = true, last_seen_at = $1, status = $2 WHERE id = $3 AND user_id = $4', 
      [Date.now(), status, deviceId, user.id]);
      
    const updated = await pool.query('SELECT * FROM devices WHERE id = $1', [deviceId]);
    sendJson(res, 200, { device: publicDevice(updated.rows[0]) });
    if (shouldBroadcast) await broadcastDeviceSnapshot(user.id);
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/host/offline") {
    const body = await readBody(req);
    const deviceId = requireIdentifier(body.deviceId, "Device ID");
    const sessionId = requireIdentifier(body.sessionId, "Session ID");
    const existing = await pool.query(
      'SELECT * FROM devices WHERE id = $1 AND user_id = $2 AND session_id = $3',
      [deviceId, user.id, sessionId]
    );
    if (existing.rows.length > 0) {
      await pool.query('UPDATE devices SET online = false, status = $1, last_seen_at = $2 WHERE id = $3 AND user_id = $4', 
        ['offline', Date.now(), deviceId, user.id]);
      await broadcastDeviceSnapshot(user.id);
    }
    sendJson(res, 200, { ok: true });
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/connect/request") {
    const body = await readBody(req);
    const deviceId = requireIdentifier(body.deviceId, "Device ID");
    const clientSessionId = requireIdentifier(body.sessionId, "Session ID");
    await cleanOfflineDevices();
    const existing = await pool.query(
      'SELECT * FROM devices WHERE id = $1 AND user_id = $2 AND online = true AND last_seen_at > $3',
      [deviceId, user.id, Date.now() - OFFLINE_AFTER_MS]
    );
    if (existing.rows.length === 0) {
      sendJson(res, 404, { error: "Host is not online." });
      return;
    }
    const device = existing.rows[0];
    if (clientSessionId === device.session_id) {
      sendJson(res, 400, { error: "A host cannot connect to its own session." });
      return;
    }
    const nativeRequest = normalizeNativeRequest(body.native, req);
    if (networkMode() === "relay") {
      if (!hasTurnServer()) {
        sendJson(res, 503, { error: "Relay mode is unavailable until TURN is configured." });
        return;
      }
      if (nativeRequest) {
        sendJson(res, 400, { error: "Native SNV cannot use TURN relay; use WebRTC Adaptive." });
        return;
      }
    }
    const autoAccept = Boolean(device.auto_accept);
    const room = {
      id: id("room"),
      userId: user.id,
      hostDeviceId: device.id,
      hostSessionId: device.session_id,
      clientSessionId,
      clientName: user.name,
      quality: normalizeQuality(body.quality),
      native: nativeRequest,
      status: autoAccept ? "accepted" : "pending",
      createdAt: Date.now(),
      updatedAt: Date.now()
    };
    state.rooms.set(room.id, room);
    await pool.query(`
      INSERT INTO connection_rooms (
        id, user_id, host_device_id, host_session_id, client_session_id, client_name, quality, status, created_at, updated_at
        , native
      ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
      ON CONFLICT (id) DO UPDATE SET
        host_device_id = EXCLUDED.host_device_id,
        host_session_id = EXCLUDED.host_session_id,
        client_session_id = EXCLUDED.client_session_id,
        client_name = EXCLUDED.client_name,
        quality = EXCLUDED.quality,
        native = EXCLUDED.native,
        status = EXCLUDED.status,
        updated_at = EXCLUDED.updated_at
    `, [
      room.id,
      room.userId,
      room.hostDeviceId,
      room.hostSessionId,
      room.clientSessionId,
      room.clientName,
      room.quality,
      room.status,
      room.createdAt,
      room.updatedAt,
      room.native
    ]);
    console.log(`[CONNECT] Room ${room.id} created. Host=${room.hostSessionId} Client=${room.clientSessionId}`);
    sendJson(res, 200, { room });
    if (autoAccept) {
      await sendEvent(user.id, "connect-accepted", { room, targetSessionId: room.hostSessionId });
      await sendEvent(user.id, "connect-accepted", { room, targetSessionId: room.clientSessionId });
    } else {
      await sendEvent(user.id, "connect-request", { room, targetSessionId: room.hostSessionId });
    }
    console.log(`[CONNECT] Events sent to user ${user.id}. SSE clients: ${state.clientsByUser.get(user.id)?.size || 0}`);
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/connect/respond") {
    const body = await readBody(req);
    const room = await getRoom(String(body.roomId || ""));
    if (!room || room.userId !== user.id) {
      sendJson(res, 404, { error: "Room not found." });
      return;
    }
    if (String(body.sessionId || "") !== room.hostSessionId) {
      sendJson(res, 403, { error: "Only the host can respond." });
      return;
    }
    if (room.status !== "pending") {
      sendJson(res, 409, { error: "Connection request was already handled." });
      return;
    }
    if (typeof body.accepted !== "boolean") {
      sendJson(res, 400, { error: "accepted must be a boolean." });
      return;
    }
    const nextStatus = body.accepted ? "accepted" : "rejected";
    const updatedAt = Date.now();
    const updated = await pool.query(
      "UPDATE connection_rooms SET status = $1, updated_at = $2 WHERE id = $3 AND user_id = $4 AND status = 'pending' RETURNING *",
      [
      nextStatus,
      updatedAt,
      room.id,
      user.id
      ]
    );
    if (updated.rows.length === 0) {
      sendJson(res, 409, { error: "Connection request was already handled." });
      return;
    }
    room.status = nextStatus;
    room.updatedAt = updatedAt;
    state.rooms.set(room.id, room);
    const event = nextStatus === "accepted" ? "connect-accepted" : "connect-rejected";
    sendJson(res, 200, { room });
    await sendEvent(user.id, event, { room, targetSessionId: room.clientSessionId });
    if (nextStatus === "accepted") {
      await sendEvent(user.id, event, { room, targetSessionId: room.hostSessionId });
    }
    return;
  }

  if (req.method === "POST" && url.pathname === "/api/signaling") {
    const body = await readBody(req);
    const room = await getRoom(String(body.roomId || ""));
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
    console.log(`[SIGNAL] ${body.message?.type || '?'} from=${fromSessionId.slice(-6)} to=${targetSessionId.slice(-6)} room=${room.id.slice(-8)}`);
    await sendEvent(user.id, "signal", {
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
    const room = await getRoom(String(body.roomId || ""));
    if (room && room.userId === user.id) {
      const sessionId = requireIdentifier(body.sessionId, "Session ID");
      if (sessionId !== room.hostSessionId && sessionId !== room.clientSessionId) {
        sendJson(res, 403, { error: "Session does not belong to this room." });
        return;
      }
      const updatedAt = Date.now();
      const updated = await pool.query(
        "UPDATE connection_rooms SET status = 'closed', updated_at = $1 WHERE id = $2 AND user_id = $3 AND status <> 'closed' RETURNING id",
        [updatedAt, room.id, user.id]
      );
      room.status = "closed";
      room.updatedAt = updatedAt;
      state.rooms.set(room.id, room);
      if (updated.rows.length > 0) {
        await sendEvent(user.id, "connect-closed", { room, targetSessionId: room.hostSessionId });
        await sendEvent(user.id, "connect-closed", { room, targetSessionId: room.clientSessionId });
      }
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
  const nativeCodec = normalizeNativeCodecPreference(input.nativeCodec);
  const networkProfile = normalizeNetworkProfile(input.networkProfile, "manual");
  const resolvedNetworkProfile = normalizeNetworkProfile(input.resolvedNetworkProfile, networkProfile);
  return {
    label: String(input.label || base.label).slice(0, 30),
    width: clamp(Number(input.width || base.width), 640, 3840),
    height: clamp(Number(input.height || base.height), 360, 2160),
    fps: clamp(Number(input.fps || base.fps), 30, 120),
    bitrateMbps: clamp(Number(input.bitrateMbps || base.bitrateMbps), 4, 120),
    preferCodec: String(input.preferCodec || "H264").toUpperCase(),
    nativeCodec,
    resolvedNativeCodec: resolveNativeCodec(nativeCodec, resolvedNetworkProfile),
    networkProfile,
    resolvedNetworkProfile
  };
}

function normalizeNetworkProfile(value, fallback = "manual") {
  const profile = String(value || fallback).trim().toLowerCase();
  return ["auto", "lan", "wifi", "internet", "relay", "tailscale", "manual"].includes(profile)
    ? profile
    : fallback;
}

function normalizeNativeCodecPreference(value) {
  const codec = String(value || "h264").trim().toLowerCase();
  if (codec === "auto") return "auto";
  if (codec === "hevc" || codec === "h265" || codec === "h.265") return "hevc";
  return "h264";
}

function resolveNativeCodec(preference, profile) {
  if (preference === "h264" || preference === "hevc") return preference;
  const normalizedProfile = profile === "tailscale" || profile === "relay" ? "internet" : profile;
  return normalizedProfile === "wifi" || normalizedProfile === "internet" ? "hevc" : "h264";
}

function normalizeNativeRequest(input = {}, req) {
  const transport = input?.transport === "snv-udp" ? "snv-udp" : (input?.transport === "snv-tcp" ? "snv-tcp" : "");
  if (!transport) return null;
  const clientIp = normalizeOptionalIp(input.clientIp, req.socket.remoteAddress);
  const clientPort = clamp(Number(input.port || input.clientPort || 7777), 1, 65533);
  const controlPort = clamp(Number(input.controlPort || clientPort + 1), 1, 65535);
  const audioPort = clamp(Number(input.audioPort || clientPort + 2), 1, 65535);
  const rawToken = String(input.sessionToken || "").trim();
  const sessionToken = /^[A-Za-z0-9._~-]{16,256}$/.test(rawToken)
    ? rawToken
    : crypto.randomBytes(32).toString("hex");
  const requestedQualityProfile = normalizeNetworkProfile(input.qualityProfile, "manual");
  return {
    transport,
    clientIp,
    clientPort,
    controlPort,
    audioPort,
    qualityProfile: requestedQualityProfile === "auto" ? "manual" : requestedQualityProfile,
    sessionToken,
    clientEndpoint: `${formatEndpointHost(clientIp)}:${clientPort}`,
    controlEndpoint: `${formatEndpointHost(clientIp)}:${controlPort}`,
    audioEndpoint: `${formatEndpointHost(clientIp)}:${audioPort}`
  };
}

function formatEndpointHost(host) {
  if (host.includes(":") && !host.startsWith("[")) return `[${host}]`;
  return host;
}

function clamp(value, min, max) {
  if (!Number.isFinite(value)) return min;
  return Math.max(min, Math.min(max, Math.round(value)));
}

function serveStatic(req, res, url) {
  if (req.method !== "GET" && req.method !== "HEAD") {
    res.writeHead(405, { Allow: "GET, HEAD" });
    res.end("Method not allowed");
    return;
  }
  let decodedPath;
  try {
    decodedPath = url.pathname === "/" ? "/index.html" : decodeURIComponent(url.pathname);
  } catch {
    throw new HttpError(400, "Malformed URL path.");
  }
  const filePath = path.resolve(PUBLIC_DIR, `.${decodedPath}`);
  const relativePath = path.relative(PUBLIC_DIR, filePath);
  if (relativePath.startsWith("..") || path.isAbsolute(relativePath)) {
    throw new HttpError(403, "Forbidden.");
  }
  fs.readFile(filePath, (error, data) => {
    if (error) {
      res.writeHead(404);
      res.end("Not found");
      return;
    }
    const type = mimeTypes[path.extname(filePath).toLowerCase()] || "application/octet-stream";
    const etag = `"${crypto.createHash("sha256").update(data).digest("base64url").slice(0, 22)}"`;
    if (req.headers["if-none-match"] === etag) {
      res.writeHead(304, { ETag: etag });
      res.end();
      return;
    }
    res.writeHead(200, {
      "Content-Type": type,
      "Content-Length": data.length,
      "Cache-Control": path.extname(filePath).toLowerCase() === ".html" ? "no-cache" : "public, max-age=300",
      "Content-Security-Policy": "default-src 'self'; connect-src 'self' http: https:; img-src 'self' data:; media-src 'self' blob:; style-src 'self' 'unsafe-inline'; script-src 'self'; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
      "Referrer-Policy": "no-referrer",
      "X-Content-Type-Options": "nosniff",
      ETag: etag
    });
    res.end(req.method === "HEAD" ? undefined : data);
  });
}

function createAppServer() {
  return http.createServer(async (req, res) => {
    try {
      const url = new URL(req.url, `http://${req.headers.host || "localhost"}`);
      if (url.pathname.startsWith("/api/")) {
        await handleApi(req, res, url);
        return;
      }
      serveStatic(req, res, url);
    } catch (error) {
      if (!(error instanceof HttpError)) console.error(error);
      if (!res.headersSent) {
        const status = error instanceof HttpError ? error.status : 500;
        const message = error instanceof HttpError ? error.message : "Server error.";
        sendJson(res, status, { error: message });
      }
    }
  });
}

async function startServer(options = {}) {
  const host = options.host || HOST;
  const port = Number.isFinite(Number(options.port)) ? Number(options.port) : PORT;
  
  await initDb();
  
  const server = createAppServer();
  let sweepRunning = false;
  let maintenanceRunning = false;
  const offlineSweep = setInterval(async () => {
    if (sweepRunning) return;
    sweepRunning = true;
    try {
      await Promise.all([expireOfflineDevices(), expirePendingRooms()]);
    } catch (error) {
      console.error("Availability sweep failed:", error.message);
    } finally {
      sweepRunning = false;
    }
  }, OFFLINE_SWEEP_MS);
  const maintenance = setInterval(async () => {
    if (maintenanceRunning) return;
    maintenanceRunning = true;
    try {
      await runMaintenance();
    } catch (error) {
      console.error("Server maintenance failed:", error.message);
    } finally {
      maintenanceRunning = false;
    }
  }, MAINTENANCE_INTERVAL_MS);
  offlineSweep.unref?.();
  maintenance.unref?.();
  const clearServerTimers = () => {
    clearInterval(offlineSweep);
    clearInterval(maintenance);
  };
  server.once("close", clearServerTimers);
  server.once("error", clearServerTimers);

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
      resolve({
        server,
        host,
        port: actualPort,
        async close() {
          for (const clients of state.clientsByUser.values()) {
            for (const client of clients) client.close?.();
          }
          state.clientsByUser.clear();
          await new Promise((closeResolve, closeReject) => {
            if (!server.listening) {
              closeResolve();
              return;
            }
            server.close((error) => error ? closeReject(error) : closeResolve());
            server.closeAllConnections?.();
          });
          if (!poolClosed) {
            poolClosed = true;
            await pool.end();
          }
        }
      });
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
