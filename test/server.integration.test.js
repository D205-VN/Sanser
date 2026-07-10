const test = require("node:test");
const assert = require("node:assert/strict");
const { Pool } = require("pg");

require("dotenv").config({ quiet: true });

function normalizeDatabaseUrl(value) {
  if (!value) return "";
  const parsed = new URL(value);
  if (["prefer", "require", "verify-ca"].includes(parsed.searchParams.get("sslmode"))) {
    parsed.searchParams.set("sslmode", "verify-full");
  }
  return parsed.toString();
}

const databaseUrl = normalizeDatabaseUrl(process.env.TEST_DATABASE_URL || "");

test("server auth, approval, native metadata, and cleanup flow", {
  skip: databaseUrl ? false : "Set TEST_DATABASE_URL to run the PostgreSQL integration test."
}, async (t) => {
  const networkEnvironment = {
    NETWORK_MODE: process.env.NETWORK_MODE,
    ICE_TRANSPORT_POLICY: process.env.ICE_TRANSPORT_POLICY,
    STUN_URLS: process.env.STUN_URLS,
    TURN_URLS: process.env.TURN_URLS,
    TURN_USERNAME: process.env.TURN_USERNAME,
    TURN_CREDENTIAL: process.env.TURN_CREDENTIAL
  };
  process.env.NETWORK_MODE = "relay";
  delete process.env.ICE_TRANSPORT_POLICY;
  process.env.STUN_URLS = "stun:127.0.0.1:3478";
  process.env.TURN_URLS = "turn:127.0.0.1:3478?transport=udp";
  process.env.TURN_USERNAME = "integration-user";
  process.env.TURN_CREDENTIAL = "integration-credential";
  process.env.DATABASE_URL = databaseUrl;
  t.after(() => {
    for (const [name, value] of Object.entries(networkEnvironment)) {
      if (value === undefined) delete process.env[name];
      else process.env[name] = value;
    }
  });
  const { startServer } = require("../server");
  const database = new Pool({ connectionString: databaseUrl });
  const handle = await startServer({ host: "127.0.0.1", port: 0 });
  const baseUrl = `http://127.0.0.1:${handle.port}`;
  const suffix = `${process.pid}-${Date.now()}`;
  const email = `sanser-test-${suffix}@example.invalid`;
  let userId = "";

  t.after(async () => {
    if (userId) await database.query("DELETE FROM users WHERE id = $1", [userId]);
    await handle.close();
    await database.end();
  });

  async function request(path, options = {}, expectedStatus = 200) {
    const response = await fetch(`${baseUrl}${path}`, options);
    const payload = await response.json().catch(() => ({}));
    assert.equal(response.status, expectedStatus, `${path}: ${JSON.stringify(payload)}`);
    return payload;
  }

  await request("/api/config", {}, 401);

  const indexResponse = await fetch(`${baseUrl}/`);
  assert.equal(indexResponse.status, 200);
  assert.match(indexResponse.headers.get("content-security-policy") || "", /frame-ancestors 'none'/);
  const indexEtag = indexResponse.headers.get("etag");
  assert.ok(indexEtag);
  await indexResponse.text();
  const cachedIndex = await fetch(`${baseUrl}/`, { headers: { "if-none-match": indexEtag } });
  assert.equal(cachedIndex.status, 304);

  const registered = await request("/api/register", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ name: "Sanser Test", email, password: "integration-test-password" })
  }, 201);
  userId = registered.user.id;
  const storedToken = await database.query("SELECT token FROM tokens WHERE user_id = $1", [userId]);
  assert.match(storedToken.rows[0].token, /^sha256:[a-f0-9]{64}$/);
  assert.notEqual(storedToken.rows[0].token, registered.token);
  const headers = {
    authorization: `Bearer ${registered.token}`,
    "content-type": "application/json"
  };

  const relayConfig = await request("/api/config", { headers });
  assert.equal(relayConfig.networkMode, "relay");
  assert.equal(relayConfig.relayReady, true);
  assert.equal(relayConfig.hasTurn, true);
  assert.equal(relayConfig.iceTransportPolicy, "relay");
  assert.deepEqual(relayConfig.iceServers, [{
    urls: ["turn:127.0.0.1:3478?transport=udp"],
    username: "integration-user",
    credential: "integration-credential"
  }]);

  process.env.NETWORK_MODE = "direct";
  const directConfig = await request("/api/config", { headers });
  assert.equal(directConfig.networkMode, "direct");
  assert.equal(directConfig.hasTurn, false);
  assert.equal(directConfig.iceTransportPolicy, "all");
  assert.deepEqual(directConfig.iceServers, [{ urls: ["stun:127.0.0.1:3478"] }]);

  process.env.NETWORK_MODE = "default";
  const defaultConfig = await request("/api/config", { headers });
  assert.equal(defaultConfig.networkMode, "default");
  assert.equal(defaultConfig.hasTurn, true);
  assert.equal(defaultConfig.iceTransportPolicy, "all");
  assert.equal(defaultConfig.iceServers.length, 2);
  process.env.NETWORK_MODE = "relay";

  const cursorResult = await database.query(
    "SELECT COALESCE(MAX(id), 0) AS id FROM app_events WHERE user_id = $1",
    [userId]
  );
  const replayCursor = Number(cursorResult.rows[0].id);
  await database.query(
    "INSERT INTO app_events (user_id, event, payload, created_at) VALUES ($1, 'test-replay', $2, $3), ($1, 'test-replay', $4, $3)",
    [userId, { index: 1 }, Date.now(), { index: 2 }]
  );
  const eventController = new AbortController();
  const eventResponse = await fetch(`${baseUrl}/api/events?token=${encodeURIComponent(registered.token)}`, {
    headers: { "last-event-id": String(replayCursor) },
    signal: eventController.signal
  });
  assert.equal(eventResponse.status, 200);
  const reader = eventResponse.body.getReader();
  const decoder = new TextDecoder();
  let eventText = "";
  try {
    await Promise.race([
      (async () => {
        while (!eventText.includes('"index":2')) {
          const { done, value } = await reader.read();
          if (done) break;
          eventText += decoder.decode(value, { stream: true });
        }
      })(),
      new Promise((_, reject) => setTimeout(() => reject(new Error("SSE replay timed out.")), 3000))
    ]);
  } finally {
    eventController.abort();
    await reader.cancel().catch(() => {});
  }
  assert.match(eventText, /event: test-replay[\s\S]*"index":1/);
  assert.match(eventText, /event: test-replay[\s\S]*"index":2/);

  const hostSessionId = `sess_host_${suffix}`;
  const clientSessionId = `sess_client_${suffix}`;
  const deviceId = `dev_${suffix}`;

  const online = await request("/api/host/online", {
    method: "POST",
    headers,
    body: JSON.stringify({
      deviceId,
      sessionId: hostSessionId,
      name: "Integration Host",
      autoAccept: false
    })
  });
  assert.equal(online.device.autoAccept, false);

  await request("/api/connect/request", {
    method: "POST",
    headers,
    body: JSON.stringify({
      deviceId,
      sessionId: clientSessionId,
      native: { transport: "snv-udp", clientIp: "127.0.0.1", port: 17777 }
    })
  }, 400);

  const configuredTurnUrls = process.env.TURN_URLS;
  process.env.TURN_URLS = "";
  await request("/api/connect/request", {
    method: "POST",
    headers,
    body: JSON.stringify({ deviceId, sessionId: clientSessionId })
  }, 503);
  process.env.TURN_URLS = configuredTurnUrls;
  process.env.NETWORK_MODE = "direct";

  const pending = await request("/api/connect/request", {
    method: "POST",
    headers,
    body: JSON.stringify({
      deviceId,
      sessionId: clientSessionId,
      quality: {
        preset: "720p",
        nativeCodec: "auto",
        networkProfile: "auto",
        resolvedNetworkProfile: "internet"
      },
      native: {
        transport: "snv-udp",
        clientIp: "127.0.0.1",
        port: 17777,
        controlPort: 17778,
        audioPort: 17779,
        qualityProfile: "internet",
        sessionToken: "integration-session-token"
      }
    })
  });
  assert.equal(pending.room.status, "pending");
  assert.equal(pending.room.quality.nativeCodec, "auto");
  assert.equal(pending.room.quality.resolvedNativeCodec, "hevc");
  assert.equal(pending.room.quality.resolvedNetworkProfile, "internet");
  assert.equal(pending.room.native.qualityProfile, "internet");

  const stored = await database.query(
    "SELECT native, quality, status FROM connection_rooms WHERE id = $1",
    [pending.room.id]
  );
  assert.equal(stored.rows[0].status, "pending");
  assert.equal(stored.rows[0].native.clientEndpoint, "127.0.0.1:17777");
  assert.equal(stored.rows[0].quality.nativeCodec, "auto");
  assert.equal(stored.rows[0].quality.resolvedNativeCodec, "hevc");

  await request("/api/signaling", {
    method: "POST",
    headers,
    body: JSON.stringify({
      roomId: pending.room.id,
      sessionId: clientSessionId,
      message: { type: "offer" }
    })
  }, 404);

  await request("/api/connect/respond", {
    method: "POST",
    headers,
    body: JSON.stringify({ roomId: pending.room.id, sessionId: clientSessionId, accepted: true })
  }, 403);

  const accepted = await request("/api/connect/respond", {
    method: "POST",
    headers,
    body: JSON.stringify({ roomId: pending.room.id, sessionId: hostSessionId, accepted: true })
  });
  assert.equal(accepted.room.status, "accepted");

  await request("/api/connect/respond", {
    method: "POST",
    headers,
    body: JSON.stringify({ roomId: pending.room.id, sessionId: hostSessionId, accepted: true })
  }, 409);

  await request("/api/connect/close", {
    method: "POST",
    headers,
    body: JSON.stringify({ roomId: pending.room.id, sessionId: clientSessionId })
  });

  const automaticHost = await request("/api/host/online", {
    method: "POST",
    headers,
    body: JSON.stringify({
      deviceId,
      sessionId: hostSessionId,
      name: "Integration Host",
      autoAccept: true
    })
  });
  assert.equal(automaticHost.device.autoAccept, true);
  const automaticRoom = await request("/api/connect/request", {
    method: "POST",
    headers,
    body: JSON.stringify({ deviceId, sessionId: clientSessionId })
  });
  assert.equal(automaticRoom.room.status, "accepted");
  await request("/api/connect/close", {
    method: "POST",
    headers,
    body: JSON.stringify({ roomId: automaticRoom.room.id, sessionId: hostSessionId })
  });

  await request("/api/host/offline", {
    method: "POST",
    headers,
    body: JSON.stringify({ deviceId, sessionId: hostSessionId })
  });
  const devices = await request("/api/devices", { headers });
  assert.equal(devices.devices.find((device) => device.id === deviceId).online, false);

  const loggedIn = await request("/api/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email, password: "integration-test-password" })
  });
  assert.ok(loggedIn.token);
  await request("/api/logout", {
    method: "POST",
    headers: { authorization: `Bearer ${loggedIn.token}` }
  });

  for (let attempt = 0; attempt < 8; attempt += 1) {
    await request("/api/login", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ email: `missing-${suffix}@example.invalid`, password: "wrong-password" })
    }, 401);
  }
  await request("/api/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email: `missing-${suffix}@example.invalid`, password: "wrong-password" })
  }, 429);

  for (let attempt = 0; attempt < 4; attempt += 1) {
    await request("/api/register", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({})
    }, 400);
  }
  await request("/api/register", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({})
  }, 429);
});
