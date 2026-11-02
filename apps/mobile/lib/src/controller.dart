import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'api.dart';
import 'native_engine.dart';
import 'preferences.dart';
import 'direct_connection.dart';

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
  final Stopwatch _diagnosticClock = Stopwatch()..start();
  final List<Json> _diagnosticSamples = [];
  int? _lastVideoFrames, _lastVideoAt;
  Json diagnosticReport() => {
    'schemaVersion': 1,
    'version': mobileVersion,
    'platform': platform,
    'transport': selectedTransport,
    'videoBitrateMbps': preferences.videoBitrateMbps,
    'samples': List<Json>.from(_diagnosticSamples),
  };
  MobilePreferences preferences = const MobilePreferences();
  Future<void> _preferenceWrites = Future.value();
  Future<void>? _disconnecting;
  DirectConnection? _direct;
  String selectedTransport = 'relay';
  String? routeNotice;
  Json connectionDiagnostics = {};
  bool _disposed = false;
  Timer? _timer;
  StreamSubscription<Json>? _events;
  bool _refreshing = false;
  int _generation = 0;
  MobileController(this.api, this.engine);
  String get platform =>
      defaultTargetPlatform == TargetPlatform.iOS ? 'iOS' : 'Android';
  Future<void> initialize() async {
    try {
      final raw = await api.secrets.read('mobile.preferences');
      if (raw != null) {
        preferences = MobilePreferences.fromJson(
          Map<String, dynamic>.from(jsonDecode(raw) as Map),
        );
      }
    } catch (_) {
      /* Invalid/old preferences fall back to safe defaults. */
    }
    try {
      capabilities = await engine.capabilities();
      _events = engine.updates.listen(
        _nativeEvent,
        onError: (Object e) {
          error = 'Mất kết nối engine native.';
          phase = SessionPhase.failed;
          unawaited(disconnect(keepError: true));
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
          error = 'Máy tính đã từ chối hoặc phiên đã kết thúc.';
          await disconnect(keepError: true);
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
    if (preferences.networkMode == 'direct' && capabilities['direct'] != true) {
      error =
          'Bản build này chưa có Direct. Cập nhật ứng dụng hoặc chọn Relay.';
      notifyListeners();
      return;
    }
    final generation = ++_generation;
    busy = true;
    error = null;
    sound = preferences.soundOnConnect;
    notifyListeners();
    try {
      final result = await api.request(
        '/api/v2/sessions',
        method: 'POST',
        body: {
          'hostDeviceId': device['id'],
          'requesterDeviceId': deviceId,
          'networkMode': capabilities['direct'] == true
              ? preferences.networkMode
              : 'relay',
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
        error = e.toString();
        await disconnect(keepError: true);
      }
    } finally {
      if (generation == _generation) busy = false;
      notifyListeners();
    }
  }

  Future<void> _start(Json session, int generation) async {
    try {
      _diagnosticSamples.clear();
      _lastVideoAt = null;
      _lastVideoFrames = null;
      _diagnosticClock.reset();
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
      final mode =
          session['networkMode'] as String? ??
          (capabilities['direct'] == true ? preferences.networkMode : 'relay');
      Json route = {'transport': 'relay'};
      routeNotice = null;
      connectionDiagnostics = {};
      if (mode != 'relay') {
        if (capabilities['direct'] != true) {
          if (mode == 'direct') {
            throw const ApiFailure(0, 'Bản build này chưa hỗ trợ Direct.');
          }
        } else {
          final direct = DirectConnection(engine);
          _direct = direct;
          try {
            route = await direct.connect(
              api: api,
              sessionId: id as String,
              deviceId: deviceId,
              peerDeviceId: credentials['peerDeviceId'] as String,
              sessionToken: credentials['sessionToken'] as String,
              accessToken: accessToken,
              readyAt: active?['requesterReadyAt'],
            );
            connectionDiagnostics = direct.diagnostics;
          } catch (_) {
            connectionDiagnostics = direct.diagnostics;
            final reason = direct.failureReason;
            await direct.cancel();
            if (generation != _generation) return;
            _direct = null;
            if (mode == 'direct') {
              throw ApiFailure(0, 'Không thiết lập được Direct. $reason');
            }
            routeNotice = 'Đã chuyển sang Relay. $reason';
          }
        }
      }
      if (generation != _generation) return;
      selectedTransport = route['transport'] as String;
      notifyListeners();
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
        'audioMuted': !sound,
        'videoBitrateMbps': preferences.videoBitrateMbps,
        'baselineRttMs': (route['probe'] as Map?)?['medianMs'] ?? 0.0,
        'transport': selectedTransport,
        'endpoint': route['endpoint'],
        'localPort': route['localPort'] ?? 0,
      });
    } catch (e) {
      if (generation == _generation) {
        error = e.toString();
        await disconnect(keepError: true);
      }
    }
  }

  Future<void> respond(Json group, Json member, bool accept) async {
    if (active != null || busy) return;
    final generation = ++_generation;
    busy = true;
    error = null;
    notifyListeners();
    try {
      final updated = await api.request(
        '/api/v2/sharing/${group['id']}/members/${member['id']}/respond',
        method: 'POST',
        body: {'viewerDeviceId': deviceId, 'accept': accept},
      );
      if (generation != _generation) return;
      if (accept) {
        sound = preferences.soundOnConnect;
        final members = updated['members'] as List? ?? [];
        final joined = members
            .whereType<Json>()
            .where((m) => m['id'] == member['id'])
            .first;
        final session = await api.request(
          '/api/v2/sessions/${joined['sessionId']}',
        );
        if (generation != _generation) return;
        active = {
          ...session,
          'networkMode':
              session['networkMode'] ?? group['networkMode'] ?? 'auto',
          'sharingId': group['id'],
          'sharingViewerId': member['id'],
        };
        await _start(active!, generation);
      }
      await refresh();
    } catch (e) {
      if (generation == _generation) {
        error = e.toString();
        await disconnect(keepError: true);
      }
    } finally {
      if (generation == _generation) busy = false;
      notifyListeners();
    }
  }

  void _nativeEvent(Json event) {
    final current = active;
    if (current == null || event['sessionId'] != current['id']) return;
    final frames = event['videoFrames'];
    if (frames is num) {
      final now = _diagnosticClock.elapsedMilliseconds;
      if (_lastVideoFrames != null &&
          _lastVideoAt != null &&
          now - _lastVideoAt! >= 250 &&
          frames >= _lastVideoFrames!) {
        event = {
          ...event,
          'videoFps':
              (frames - _lastVideoFrames!) * 1000 / (now - _lastVideoAt!),
        };
      }
      // Dimension-only events do not advance the one-second sampling clock.
      if (_lastVideoAt == null || now - _lastVideoAt! >= 250) {
        _lastVideoFrames = frames.toInt();
        _lastVideoAt = now;
      }
    }
    if (event['rttMs'] is num) {
      final sample = <String, dynamic>{
        'atMs': _diagnosticClock.elapsedMilliseconds,
      };
      for (final key in [
        'rttMs',
        'videoFps',
        'videoPackets',
        'videoFrames',
        'nackRequests',
        'lostFragments',
        'referenceDrops',
        'audioDecoded',
        'audioConcealed',
        'audioLate',
      ]) {
        final value = event[key];
        if (value is num && value.isFinite) sample[key] = value;
      }
      _diagnosticSamples.add(sample);
      if (_diagnosticSamples.length > 120) _diagnosticSamples.removeAt(0);
    }
    telemetry = {...telemetry, ...event};
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
    if (_disconnecting != null) return _disconnecting;
    final completion = Completer<void>();
    _disconnecting = completion.future;
    try {
      await _disconnect(keepError: keepError);
    } finally {
      _disconnecting = null;
      completion.complete();
    }
  }

  Future<void> _disconnect({required bool keepError}) async {
    final old = active;
    final direct = _direct;
    _direct = null;
    if (direct != null && !direct.active) unawaited(direct.cancel());
    final generation = ++_generation;
    busy = true;
    active = null;
    phase = keepError ? SessionPhase.failed : SessionPhase.idle;
    if (!keepError) error = null;
    voiceJoined = false;
    microphone = false;
    deafened = false;
    telemetry = {};
    notifyListeners();
    try {
      await engine.disconnect();
    } catch (_) {
      /* server cleanup must still run */
    }
    await direct?.cancel();
    try {
      if (old != null) {
        await api.request(
          '/api/v2/sessions/${old['id']}/disconnect',
          method: 'POST',
        );
      }
    } catch (e) {
      error ??= e.toString();
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
    try {
      await engine.mute(!value);
      sound = value;
    } catch (e) {
      error = e.toString();
    }
    notifyListeners();
  }

  Future<void> setPreferences(MobilePreferences value) async {
    preferences = value;
    notifyListeners();
    final encoded = jsonEncode(value.toJson());
    _preferenceWrites = _preferenceWrites
        .catchError((Object _) {})
        .then((_) => api.secrets.write('mobile.preferences', encoded));
    try {
      await _preferenceWrites;
    } catch (_) {
      error = 'Không lưu được cài đặt. Vui lòng thử lại.';
      notifyListeners();
    }
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
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _timer?.cancel();
    _events?.cancel();
    api.close();
    super.dispose();
  }
}
