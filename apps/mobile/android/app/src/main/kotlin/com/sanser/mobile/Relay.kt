package com.sanser.mobile

import com.google.crypto.tink.aead.internal.InsecureNonceXChaCha20Poly1305
import okhttp3.*
import okio.ByteString
import okhttp3.HttpUrl.Companion.toHttpUrl
import okio.ByteString.Companion.toByteString
import org.json.JSONObject
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.nio.ByteBuffer
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.UUID
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** Existing desktop SNR1 envelope, including session binding and replay checks. */
internal class RelayCipher(session: String, own: String, peer: String, token: String) {
    private val sessionId = uuid(session)
    private val ownId = uuid(own)
    private val peerId = uuid(peer)
    private val cipher = InsecureNonceXChaCha20Poly1305(MessageDigest.getInstance("SHA-256").run {
        update("sanser-relay-frame-key-v1\u0000".toByteArray(Charsets.UTF_8)); update(sessionId); digest(token.toByteArray(Charsets.UTF_8))
    })
    private val prefix = ByteArray(16).also { SecureRandom().nextBytes(it) }
    private var sequence = 0L
    private var remotePrefix: ByteArray? = null
    private var received = -1L
    @Synchronized fun seal(packet: ByteArray): ByteArray {
        require(packet.size in 1..65476 && sequence < Long.MAX_VALUE)
        val header = ByteBuffer.allocate(44).put("SNR1".toByteArray()).put(ownId).put(prefix).putLong(sequence++).array()
        return header + cipher.encrypt(header.copyOfRange(20,44), packet, header + sessionId)
    }
    @Synchronized fun open(frame: ByteArray): ByteArray? {
        if (frame.size !in 61..65536 || !frame.copyOfRange(0,4).contentEquals("SNR1".toByteArray()) || !frame.copyOfRange(4,20).contentEquals(peerId)) return null
        val seq = ByteBuffer.wrap(frame,36,8).long
        val noncePrefix = frame.copyOfRange(20,36)
        if (seq < 0 || seq <= received || (remotePrefix != null && !remotePrefix!!.contentEquals(noncePrefix))) return null
        val header = frame.copyOfRange(0,44)
        val packet = try { cipher.decrypt(frame.copyOfRange(20,44), frame.copyOfRange(44,frame.size), header + sessionId) } catch (_: Exception) { return null }
        // Ordered WSS needs a monotonic fence. Never advance it before AEAD succeeds.
        remotePrefix = noncePrefix; received = seq
        return packet
    }
    private fun uuid(value: String): ByteArray = UUID.fromString(value).let { ByteBuffer.allocate(16).putLong(it.mostSignificantBits).putLong(it.leastSignificantBits).array() }
}

internal class Relay(private val configuration: Map<String, Any?>, private val ready: (String) -> Unit, private val failed: (String) -> Unit) : AutoCloseable {
    private val running = AtomicBoolean(true)
    private val local = DatagramSocket(0, InetAddress.getByName("127.0.0.1")).apply { soTimeout = 250 }
    private val cipher = RelayCipher(configuration["sessionId"] as String, configuration["deviceId"] as String, configuration["peerDeviceId"] as String, configuration["sessionToken"] as String)
    private val http = OkHttpClient.Builder().pingInterval(15,TimeUnit.SECONDS).readTimeout(0,TimeUnit.SECONDS).build()
    private var remotePort: Int? = null
    private var ws: WebSocket? = null
    private val sender = Thread({
        val buffer = ByteArray(65536)
        while (running.get()) {
            val packet = DatagramPacket(buffer,buffer.size)
            try {
                local.receive(packet)
                if (!packet.address.isLoopbackAddress || packet.length == 0) continue
                val expected = remotePort
                if (expected != null && expected != packet.port) continue
                remotePort = packet.port
                val socket = ws ?: continue
                if (socket.queueSize() > 2 * 1024 * 1024 || !socket.send(cipher.seal(packet.data.copyOfRange(packet.offset,packet.offset+packet.length)).toByteString())) {
                    fail("Hàng đợi relay vượt giới hạn. Vui lòng kết nối lại.")
                }
            } catch (_: java.net.SocketTimeoutException) { }
            catch (_: Exception) { if (running.get()) fail("Không thể chuyển tiếp gói tin native.") }
        }
    }, "SanserRelayTX")
    fun start() {
        val base = (configuration["serverUrl"] as String).toHttpUrlStrict()
        val url = base.newBuilder().addPathSegments("api/v2/relay")
            .addQueryParameter("sessionId",configuration["sessionId"] as String)
            .addQueryParameter("deviceId",configuration["deviceId"] as String).build()
        val token = configuration["accessToken"] as String
        require(!token.contains('\r') && !token.contains('\n'))
        val request = Request.Builder().url(url).header("Sec-WebSocket-Protocol", "sanser-relay-v1, bearer.$token").build()
        sender.start()
        ws = http.newWebSocket(request, object: WebSocketListener() {
            private val announced = AtomicBoolean(false)
            override fun onOpen(webSocket: WebSocket, response: Response) {
                if (response.header("Sec-WebSocket-Protocol") != "sanser-relay-v1") fail("Máy chủ không hỗ trợ relay mobile.")
            }
            override fun onMessage(webSocket: WebSocket, text: String) {
                if (text.length > 16384) return fail("Phản hồi relay không hợp lệ.")
                val type = try { JSONObject(text).optString("type") } catch (_: Exception) { "" }
                if (type == "relay.peerReady" && announced.compareAndSet(false,true) && running.get()) ready("127.0.0.1:${local.localPort}")
                if (type == "relay.peerOffline") fail("Máy tính đã rời phiên.")
            }
            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                if (!running.get()) return
                val packet = cipher.open(bytes.toByteArray()) ?: return
                val port = remotePort ?: return
                if (packet.size > 65476) return
                try { local.send(DatagramPacket(packet,packet.size,InetAddress.getByName("127.0.0.1"),port)) }
                catch (_: Exception) { if (running.get()) fail("Mất kết nối với engine native.") }
            }
            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) { if (running.get()) fail("Kết nối relay đã ngắt. Vui lòng thử lại.") }
            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { webSocket.close(code,null);if(running.get()) fail("Máy chủ đã đóng kết nối relay.") }
        })
    }
    private fun fail(message: String) { if(running.compareAndSet(true,false)) { local.close();ws?.cancel();failed(message) } }
    override fun close() {
        running.set(false);local.close();ws?.close(1000,"Session ended");ws?.cancel()
        if (Thread.currentThread() != sender && sender.isAlive) sender.join(1000)
        http.dispatcher.executorService.shutdown();http.connectionPool.evictAll()
    }
    private fun String.toHttpUrlStrict(): HttpUrl {
        val url = this.toHttpUrl()
        require(url.isHttps && url.username.isEmpty() && url.password.isEmpty() && url.query == null && url.fragment == null)
        return url
    }
}
