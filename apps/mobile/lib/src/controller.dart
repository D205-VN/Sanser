import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'api.dart';
import 'native_engine.dart';

enum SessionPhase { idle, approval, connecting, streaming, failed }

class MobileController extends ChangeNotifier {
  final SanserApi api;
  final NativeEngine engine;
  Json? account;
  Json? active;
  Json capabilities = {};
  List<Json> devices = [];
  List<Json> sharing = [];
  String deviceId = '';
  String? error;
  bool busy = false;
  bool sound = true;
  bool voiceJoined = false, microphone = false, deafened = false;
  SessionPhase phase = SessionPhase.idle;
  Json telemetry = {};
  Timer? _timer;
  StreamSubscription<Json>? _events;
  bool _refreshing = false;
  int _generation = 0;
  MobileController(this.api, this.engine);
  String get platform =>
      defaultTargetPlatform == TargetPlatform.iOS ? 'iOS' : 'Android';
  Future<void> initialize() async {
    try {
      capabilities = await engine.capabilities();
      _events = engine.updates.listen(
        _nativeEvent,
        onError: (Object e) {
          error = 'Mất kết nối engine native.';
          phase = SessionPhase.failed;
          notifyListeners();
        },
      );
    } on MissingPluginException {
      capabilities = {'available': false};
    }
    await api.loadServer(
      const String.fromEnvironment(
        'SANSER_SERVER_URL',
        defaultValue: 'https://sanser.onrender.com',
      ),
    );
    try {
      if (await api.restore()) await _signedIn();
    } catch (e) {
      error = e.toString();
    }
    notifyListeners();
  }

  Future<void> login(
    String email,
    String password,
    String server, {
    bool register = false,
  }) async {
    busy = true;
    error = null;
    notifyListeners();
    try {
      api.useServer(server);
      await api.login(
        email,
        password,
        deviceName: 'Sanser $platform',
        platform: platform,
        register: register,
      );
      await _signedIn();
    } catch (e) {
      error = e.toString();
    } finally {
      busy = false;
      notifyListeners();
    }
  }

  Future<void> _signedIn() async {
    account = await api.request('/api/v2/account');
    deviceId = await api.identity('client-device');
    if (capabilities['available'] == true) {
      await api.request(
        '/api/v2/devices/register',
        method: 'POST',
        body: {
          'id': deviceId,
          'computerId': await api.identity('computer'),
          'deviceRole': 'client',
          'crossPlatform': true,
          'name': 'Sanser $platform',
          'platform': platform,
          'osVersion': platform,
          'gpu': 'Native H.264 decoder',
          'version': mobileVersion,
          'protocolVersion': 2,
          'codecs': ['h264'],
          'nativeTransport': true,
          'webRtc': false,
          'audio': true,
          'gamepad': false,
        },
      );
    }
    await refresh();
    _timer?.cancel();
    _timer = Timer.periodic(
      const Duration(seconds: 5),
      (_) => unawaited(refresh()),
    );
  }

  Future<void> refresh() async {
    if (account == null || _refreshing) return;
    _refreshing = true;
    final generation = _generation;
    try {
      final all = await api.list('/api/v2/devices');
      final groups = await api.list('/api/v2/sharing');
      if (generation != _generation) return;
      devices = all
          .where((d) => d['deviceRole'] == 'host' && d['id'] != deviceId)
          .toList();
      sharing = groups;
      if (capabilities['available'] == true) {
        await api.request(
          '/api/v2/devices/heartbeat',
          method: 'POST',
          body: {
            'deviceId': deviceId,
            'streaming': phase == SessionPhase.streaming,
          },
        );
      }
      if (active != null && phase == SessionPhase.approval) {
        final session = await api.request('/api/v2/sessions/${active!['id']}');
        if (generation != _generation) return;
        active = {...active!, ...session};
        if ((session['status'] ?? session['state']) == 'accepted') {
          await _start(active!, generation);
        } else if ([
          'rejected',
          'expired',
          'disconnected',
          'closed',
          'failed',
        ].contains(session['status'] ?? session['state'])) {
          phase = SessionPhase.failed;
          error = 'Máy tính đã từ chối hoặc phiên đã kết thúc.';
        }
      }
    } catch (e) {
      if (generation == _generation) error = e.toString();
    } finally {
      _refreshing = false;
      notifyListeners();
    }
  }

  Future<void> connect(Json device, {bool viewOnly = false}) async {
    if (active != null || busy) return;
    if (capabilities['available'] != true) {
      error = 'Engine mobile chưa sẵn sàng trong bản build này.';
      notifyListeners();
      return;
    }
    final generation = ++_generation;
    busy = true;
    error = null;
    notifyListeners();
    try {
      // First mobile transport is the existing authenticated WSS relay. Do not
      // advertise a successful Direct probe or fabricate low latency metrics.
      final result = await api.request(
        '/api/v2/sessions',
        method: 'POST',
        body: {
          'hostDeviceId': device['id'],
          'requesterDeviceId': deviceId,
          'networkMode': 'relay',
          'qualityProfile': 'balanced',
          'requestedCodec': 'h264',
          'accessMode': viewOnly ? 'view-only' : 'control',
        },
      );
      if (generation != _generation) {
        await api.request(
          '/api/v2/sessions/${result['id']}/disconnect',
          method: 'POST',
        );
        return;
      }
      active = result;
      phase = SessionPhase.approval;
      if ((result['status'] ?? result['state']) == 'accepted') {
        await _start(result, generation);
      }
    } catch (e) {
      if (generation == _generation) {
        phase = SessionPhase.failed;
        error = e.toString();
      }
    } finally {
      busy = false;
      notifyListeners();
    }
  }

  Future<void> _start(Json session, int generation) async {
    phase = SessionPhase.connecting;
    notifyListeners();
    final id = session['id'];
    final ready = await api.request(
      '/api/v2/sessions/$id/native-ready',
      method: 'POST',
      body: {'deviceId': deviceId},
    );
    if (generation != _generation) return;
    active = {...session, ...ready};
    final credentials = await api.request(
      '/api/v2/sessions/$id/credentials?deviceId=$deviceId',
    );
    if (generation != _generation) return;
    if (credentials['wireProtocol'] != 'snv2') {
      throw const ApiFailure(
        0,
        'Máy tính cần hỗ trợ giao thức SNV2 để kết nối mobile.',
      );
    }
    final accessToken = await api.accessToken();
    if (generation != _generation) return;
    await engine.connect({
      'serverUrl': api.base.toString(),
      'sessionId': id,
      'deviceId': deviceId,
      'peerDeviceId': credentials['peerDeviceId'],
      'accessToken': accessToken,
      'sessionToken': credentials['sessionToken'],
      'inputEnabled': credentials['accessMode'] != 'view-only',
      'sharingId': session['sharingId'],
      'viewerId': session['sharingViewerId'] ?? '',
    });
  }

  Future<void> respond(Json group, Json member, bool accept) async {
    if (active != null) return;
    busy = true;
    error = null;
    notifyListeners();
    try {
      final updated = await api.request(
        '/api/v2/sharing/${group['id']}/members/${member['id']}/respond',
        method: 'POST',
        body: {'viewerDeviceId': deviceId, 'accept': accept},
      );
      if (accept) {
        final members = updated['members'] as List? ?? [];
        final joined = members
            .whereType<Json>()
            .where((m) => m['id'] == member['id'])
            .first;
        final session = await api.request(
          '/api/v2/sessions/${joined['sessionId']}',
        );
        active = {
          ...session,
          'sharingId': group['id'],
          'sharingViewerId': member['id'],
        };
        await _start(active!, ++_generation);
      }
      await refresh();
    } catch (e) {
      phase = SessionPhase.failed;
      error = e.toString();
    } finally {
      busy = false;
      notifyListeners();
    }
  }

  void _nativeEvent(Json event) {
    final current = active;
    if (current == null || event['sessionId'] != current['id']) return;
    telemetry = event;
    switch (event['state']) {
      case 'frame':
        if (phase != SessionPhase.streaming && current['sharingId'] != null) {
          final readyAt = current['requesterReadyAt'];
          final seconds = readyAt is num
              ? readyAt.toInt()
              : readyAt is String
              ? DateTime.tryParse(readyAt)?.millisecondsSinceEpoch
              : null;
          if (seconds != null) {
            unawaited(
              api
                  .request(
                    '/api/v2/sessions/${current['id']}/native-started',
                    method: 'POST',
                    body: {
                      'deviceId': deviceId,
                      'requesterReadyAt': seconds > 10000000000
                          ? seconds ~/ 1000
                          : seconds,
                    },
                  )
                  .catchError((Object _) => <String, dynamic>{}),
            );
          }
        }
        phase = SessionPhase.streaming;
      case 'stopped':
        phase = SessionPhase.failed;
        error = event['message'] as String? ?? 'Luồng từ xa đã kết thúc.';
        unawaited(disconnect(keepError: true));
    }
    notifyListeners();
  }

  Future<void> disconnect({bool keepError = false}) async {
    final old = active;
    final generation = ++_generation;
    busy = true;
    active = null;
    voiceJoined = false;
    microphone = false;
    deafened = false;
    telemetry = {};
    try {
      await engine.disconnect();
    } catch (_) {
      /* server cleanup must still run */
    }
    try {
      if (old != null) {
        await api.request(
          '/api/v2/sessions/${old['id']}/disconnect',
          method: 'POST',
        );
      }
    } catch (e) {
      error = e.toString();
    }
    if (generation != _generation) return;
    busy = false;
    if (!keepError) {
      phase = SessionPhase.idle;
      error = null;
    }
    notifyListeners();
  }

  Future<void> setSound(bool value) async {
    await engine.mute(!value);
    sound = value;
    notifyListeners();
  }

  Future<void> setVoice({bool? joined, bool? mic, bool? deaf}) async {
    final join = joined ?? voiceJoined;
    final deafen = join && (deaf ?? deafened);
    final microphoneOn = join && !deafen && (mic ?? microphone);
    try {
      await engine.voice(
        joined: join,
        microphone: microphoneOn,
        deafened: deafen,
      );
      voiceJoined = join;
      microphone = microphoneOn;
      deafened = deafen;
      error = null;
    } catch (e) {
      error = e.toString();
    }
    notifyListeners();
  }

  Future<void> logout() async {
    _timer?.cancel();
    await disconnect();
    try {
      if (deviceId.isNotEmpty && capabilities['available'] == true) {
        await api.request(
          '/api/v2/devices/offline',
          method: 'POST',
          body: {'deviceId': deviceId},
        );
      }
    } finally {
      try {
        await api.logout();
      } finally {
        account = null;
        devices = [];
        sharing = [];
        notifyListeners();
      }
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    _events?.cancel();
    api.close();
    super.dispose();
  }
}
