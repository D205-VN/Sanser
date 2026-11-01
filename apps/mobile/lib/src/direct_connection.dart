import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:uuid/uuid.dart';
import 'api.dart';
import 'native_engine.dart';

/// Authenticated candidate signaling only; all UDP probes/media stay native.
class DirectConnection {
  final NativeEngine engine;
  final Future<WebSocket> Function(Uri, List<String>) _open;
  final String attemptId = const Uuid().v4();
  WebSocket? _socket;
  Completer<Json>? _pending;
  bool _cancelled = false;
  bool active = false;
  String _stage = 'prepare';
  bool? _stunSucceeded;
  int _localCandidates = 0, _remoteCandidates = 0;
  bool _acknowledged = false;
  Json get diagnostics => {
    'stage': _stage,
    'stunSucceeded': _stunSucceeded,
    'localCandidates': _localCandidates,
    'remoteCandidates': _remoteCandidates,
    'peerAcknowledged': _acknowledged,
  };
  // Deliberately avoid raw exception text: socket errors can contain the
  // signaling URL or auth information. Report measured progress instead.
  String get failureReason => switch (_stage) {
    'prepare' => 'Thông tin chuẩn bị phiên Direct chưa hợp lệ.',
    'gather' => 'Không thu thập được địa chỉ UDP trên điện thoại.',
    'signaling' when _remoteCandidates == 0 =>
      'Chưa nhận được địa chỉ Direct từ máy tính.',
    'signaling' when !_acknowledged =>
      'Máy tính chưa xác nhận địa chỉ Direct của điện thoại.',
    'signaling' => 'Trao đổi thông tin Direct chưa hoàn tất.',
    'check' when _stunSucceeded == false =>
      'Chưa xác minh được đường UDP; điện thoại chưa lấy được địa chỉ public qua STUN.',
    'check' =>
      'Đã trao đổi địa chỉ nhưng chưa xác minh được đường UDP giữa hai máy.',
    'activate' => 'Đã xác minh Direct nhưng chưa mở được cầu nối tới engine.',
    _ => 'Direct chưa hoàn tất.',
  };
  DirectConnection(
    this.engine, {
    Future<WebSocket> Function(Uri, List<String>)? open,
  }) : _open =
           open ??
           ((url, protocols) =>
               WebSocket.connect(url.toString(), protocols: protocols));

  Future<void> _close(WebSocket? socket) async {
    if (socket == null) return;
    try {
      await socket.close().timeout(const Duration(seconds: 1));
    } catch (_) {
      // A broken signaling close must not block native stop or lose Direct.
    }
  }

  Future<void> cancel() async {
    _cancelled = true;
    final pending = _pending;
    if (pending != null && !pending.isCompleted) {
      pending.completeError(const ApiFailure(0, 'Đã hủy kết nối Direct.'));
    }
    unawaited(_close(_socket));
    try {
      await engine.p2p({'operation': 'close', 'attemptId': attemptId});
    } catch (_) {
      /* Closing an already released route is harmless. */
    }
  }

  Future<Json> connect({
    required SanserApi api,
    required String sessionId,
    required String deviceId,
    required String peerDeviceId,
    required String sessionToken,
    required String accessToken,
    required Object? readyAt,
  }) async {
    final generation = readyAt is num
        ? readyAt.toInt()
        : int.tryParse('$readyAt') ??
              ((DateTime.tryParse('$readyAt')?.millisecondsSinceEpoch ?? 0) ~/
                  1000);
    if (generation < 1 || generation > 0xffffffff) {
      throw const ApiFailure(0, 'Phiên chưa có thế hệ kết nối Direct hợp lệ.');
    }
    List<String> stun = [];
    try {
      final ice = await api.request('/api/v2/network/ice');
      stun = <String>{
        for (final server in (ice['iceServers'] as List? ?? []))
          if (server is Map) ...[
            for (final value
                in (server['urls'] is List
                    ? server['urls'] as List
                    : [server['urls']]))
              if (value is String && value.startsWith('stun:')) value,
          ],
      }.take(3).toList();
    } catch (_) {
      /* LAN can still work without a reachable STUN service. */
    }
    if (_cancelled) throw const ApiFailure(0, 'Đã hủy kết nối Direct.');
    _stage = 'gather';
    final gathered = await engine.p2p({
      'operation': 'gather',
      'attemptId': attemptId,
      'sessionId': sessionId,
      'generation': generation,
      'stunServers': stun,
    });
    _stunSucceeded = gathered['stunSucceeded'] as bool?;
    if (_cancelled) {
      await cancel();
      throw const ApiFailure(0, 'Đã hủy kết nối Direct.');
    }
    final candidates = gathered['candidates'] as List? ?? [];
    _localCandidates = candidates.length;
    if (candidates.isEmpty) {
      throw const ApiFailure(
        0,
        'Không tìm thấy địa chỉ Direct trên điện thoại.',
      );
    }
    _stage = 'signaling';
    final url = api.base.replace(
      scheme: 'wss',
      path: '${api.base.path}/api/v2/signaling',
      queryParameters: {'deviceId': deviceId},
    );
    var expired = false;
    final socket = await _open(url, ['sanser-v2', 'bearer.$accessToken'])
        .then((socket) async {
          if (_cancelled || expired) {
            await _close(socket);
            throw const ApiFailure(0, 'Đã hủy kết nối Direct.');
          }
          return socket;
        })
        .timeout(
          const Duration(seconds: 10),
          onTimeout: () {
            expired = true;
            throw const ApiFailure(
              0,
              'Hết thời gian mở kênh thương lượng Direct.',
            );
          },
        );
    _socket = socket;
    if (_cancelled) {
      await _close(socket);
      throw const ApiFailure(0, 'Đã hủy kết nối Direct.');
    }
    if (socket.protocol != 'sanser-v2') {
      await _close(socket);
      throw const ApiFailure(0, 'Máy chủ không hỗ trợ thương lượng Direct.');
    }
    final result = Completer<Json>();
    _pending = result;
    Timer? retry;
    var acknowledged = false, checking = false;
    List? remote;
    void fail(Object error) {
      if (!result.isCompleted) result.completeError(error);
    }

    void send(String type, Json payload) {
      if (_cancelled || socket.readyState != WebSocket.open) return;
      try {
        socket.add(
          jsonEncode({
            'sessionId': sessionId,
            'targetDeviceId': peerDeviceId,
            'type': type,
            'payload': payload,
          }),
        );
      } catch (_) {
        fail(const ApiFailure(0, 'Kênh thương lượng Direct đã ngắt.'));
      }
    }

    void sendCandidates() {
      if (result.isCompleted || acknowledged || _cancelled) return;
      send('p2p.candidates', {
        'generation': generation,
        'candidates': candidates,
      });
    }

    Future<void> check() async {
      if (checking ||
          !acknowledged ||
          remote == null ||
          result.isCompleted ||
          _cancelled) {
        return;
      }
      checking = true;
      _stage = 'check';
      retry?.cancel();
      try {
        final selected = await engine.p2p({
          'operation': 'check',
          'attemptId': attemptId,
          'deviceId': deviceId,
          'peerDeviceId': peerDeviceId,
          'sessionToken': sessionToken,
          'remoteCandidates': remote,
        });
        if (_cancelled || result.isCompleted) return;
        _stage = 'activate';
        final bridge = await engine.p2p({
          'operation': 'activate',
          'attemptId': attemptId,
        });
        if (_cancelled || result.isCompleted) return;
        active = true;
        _stage = 'active';
        result.complete({...selected, ...bridge});
      } catch (error) {
        fail(error);
      }
    }

    final subscription = socket.listen(
      (Object? message) {
        if (result.isCompleted ||
            message is! String ||
            message.length > 65536) {
          return;
        }
        try {
          final data = Map<String, dynamic>.from(jsonDecode(message) as Map);
          if (data['type'] == 'error') {
            final code = (data['error'] as Map?)?['code'];
            if ([
              'target_offline',
              'target_backpressure',
              'invalid_signal_type',
            ].contains(code)) {
              return;
            }
            fail(const ApiFailure(0, 'Máy chủ từ chối thương lượng Direct.'));
            return;
          }
          if (data['sessionId'] != sessionId ||
              data['senderDeviceId'] != peerDeviceId) {
            return;
          }
          final payload = data['payload'] as Map?;
          if (payload?['generation'] != generation) return;
          if (data['type'] == 'p2p.candidates') {
            final incoming = payload?['candidates'];
            if (incoming is! List || incoming.isEmpty || incoming.length > 16) {
              return;
            }
            remote ??= incoming;
            _remoteCandidates = remote!.length;
            send('p2p.candidatesAck', {'generation': generation});
            send('p2p.gatheringComplete', {'generation': generation});
          } else if ([
            'p2p.candidatesAck',
            'p2p.gatheringComplete',
          ].contains(data['type'])) {
            acknowledged = true;
            _acknowledged = true;
          }
          unawaited(check());
        } catch (_) {
          fail(
            const ApiFailure(0, 'Phản hồi thương lượng Direct không hợp lệ.'),
          );
        }
      },
      onError: (Object _) {
        if (!checking) {
          fail(const ApiFailure(0, 'Kênh thương lượng Direct đã ngắt.'));
        }
      },
      onDone: () {
        if (!checking) {
          fail(const ApiFailure(0, 'Kênh thương lượng Direct đã đóng.'));
        }
      },
    );
    retry = Timer.periodic(
      const Duration(milliseconds: 600),
      (_) => sendCandidates(),
    );
    final deadline = Timer(
      const Duration(seconds: 20),
      () => fail(
        const ApiFailure(
          0,
          'Không thiết lập được Direct. Mạng có thể chặn UDP hoặc cô lập các thiết bị.',
        ),
      ),
    );
    sendCandidates();
    try {
      return await result.future;
    } finally {
      deadline.cancel();
      retry.cancel();
      await subscription.cancel();
      await _close(socket);
      _pending = null;
    }
  }
}
