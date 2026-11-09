import 'dart:async';
import 'dart:convert';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'api.dart';
import 'native_engine.dart';
import 'preferences.dart';
import 'direct_connection.dart';

enum SessionPhase {
  idle,
  approval,
  connecting,
  reconnecting,
  streaming,
  failed,
}

class _MobileRouteChanged implements Exception {
  const _MobileRouteChanged();
}

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
  Json networkDetails = {};
  String get networkLabel => switch (networkDetails['networkKind']) {
    'wifi' => 'Wi‑Fi',
    'cellular' => 'Di động',
    'mixed' => 'Wi‑Fi/Di động',
    'ethernet' => 'Ethernet',
    'offline' => 'Mất mạng',
    _ => '',
  };
  final Stopwatch _diagnosticClock = Stopwatch()..start();
  final List<Json> _diagnosticSamples = [];
  int? _lastVideoFrames, _lastVideoAt;
  Json _lastMediaCounters = {};
  Json diagnosticReport() => {
    'schemaVersion': 1,
    'version': mobileVersion,
    'platform': platform,
    'transport': selectedTransport,
    'videoBitrateMbps': preferences.videoBitrateMbps,
    'direct': Map<String, dynamic>.from(connectionDiagnostics),
    'network': Map<String, dynamic>.from(networkDetails),
    'samples': List<Json>.from(_diagnosticSamples),
    'recovery': {'networkChanges': _pathRevision, 'attempts': recoveryAttempts},
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
  int get engineAttempt => _engineAttempt;
  int mediaRevision = 0, recoveryAttempts = 0;
  int _engineAttempt = 0,
      _pathRevision = 0,
      _networkRevision = 0,
      _recoveryEpisode = 0;
  bool _acceptNative = true, _networkAvailable = true, _foreground = true;
  bool _recovering = false, _recoveryRunning = false, _retryAfterRun = false;
  Timer? _recoveryTimer, _recoveryDeadline;
  bool get recovering => _recovering;
  String get recoveryStatus => !_networkAvailable
      ? 'Mất mạng · đang chờ Wi-Fi hoặc dữ liệu di động…'
      : 'Mạng đã thay đổi · đang xác minh lại kết nối ($recoveryAttempts/2)…';
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
    final signedInAccount = await api.request('/api/v2/account');
    if (capabilities['available'] == true) {
      deviceId = await api.registerClientDevice(
        {
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
        accountId:
            (signedInAccount['id'] ?? signedInAccount['email']) as String? ??
            '',
      );
    }
    account = signedInAccount;
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

  Future<void> _start(Json session, int generation, {int? pathRevision}) async {
    bool current() {
      if (_disposed || generation != _generation) return false;
      if (pathRevision != null && pathRevision != _pathRevision) {
        throw const _MobileRouteChanged();
      }
      return true;
    }

    try {
      if (pathRevision == null) {
        _diagnosticSamples.clear();
        _diagnosticClock.reset();
      }
      _lastVideoAt = null;
      _lastVideoFrames = null;
      _lastMediaCounters = {};
      phase = pathRevision == null
          ? SessionPhase.connecting
          : SessionPhase.reconnecting;
      notifyListeners();
      final id = session['id'];
      final ready = await api.request(
        '/api/v2/sessions/$id/native-ready',
        method: 'POST',
        body: {'deviceId': deviceId},
      );
      if (!current()) return;
      active = {...session, ...ready};
      final credentials = await api.request(
        '/api/v2/sessions/$id/credentials?deviceId=$deviceId',
      );
      if (!current()) return;
      if (credentials['wireProtocol'] != 'snv2') {
        throw const ApiFailure(
          0,
          'Máy tính cần hỗ trợ giao thức SNV2 để kết nối mobile.',
        );
      }
      final accessToken = await api.accessToken();
      if (!current()) return;
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
            if (!current()) return;
            _direct = null;
            if (mode == 'direct') {
              throw ApiFailure(0, 'Không thiết lập được Direct. $reason');
            }
            routeNotice = 'Đã chuyển sang Relay. $reason';
          }
        }
      }
      if (!current()) return;
      selectedTransport = route['transport'] as String;
      notifyListeners();
      _acceptNative = true;
      final attempt = ++_engineAttempt;
      await engine.connect({
        'engineAttempt': attempt,
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
      current();
    } catch (e) {
      if (pathRevision != null) rethrow;
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
    if (_disposed) return;
    if (event['state'] == 'network') {
      _networkEvent(event);
      return;
    }
    // High-rate local controller snapshots have their own focused consumer.
    // They are not session telemetry or a reason to rebuild the workspace.
    if (event['state'] == 'controller') return;
    final current = active;
    if (current == null || event['sessionId'] != current['id']) return;
    if (!_acceptNative ||
        (capabilities['networkRecovery'] == true &&
            event['engineAttempt'] != _engineAttempt)) {
      return;
    }
    final frames = event['videoFrames'];
    if (frames is num) {
      // Native health is sampled independently of Flutter's event timer.
      // Repeated snapshots must not become artificial 0/120 FPS oscillations.
      final nativeTime = event['sampleTimeMs'];
      final now = nativeTime is num && nativeTime > 0
          ? nativeTime.toInt()
          : _diagnosticClock.elapsedMilliseconds;
      if (_lastVideoFrames != null &&
          _lastVideoAt != null &&
          now - _lastVideoAt! >= 250 &&
          frames >= _lastVideoFrames!) {
        event = {
          ...event,
          'videoFps':
              (frames - _lastVideoFrames!) * 1000 / (now - _lastVideoAt!),
        };
        for (final key in ['nackRequests', 'audioConcealed', 'audioLate']) {
          final current = event[key], previous = _lastMediaCounters[key];
          if (current is num && previous is num && current >= previous) {
            event['${key}PerSecond'] =
                (current - previous) * 1000 / (now - _lastVideoAt!);
          }
        }
        final shown = event['iOSPresentedFrames'];
        final previousShown = _lastMediaCounters['iOSPresentedFrames'];
        if (shown is num && previousShown is num && shown >= previousShown) {
          event['displayFps'] =
              (shown - previousShown) * 1000 / (now - _lastVideoAt!);
        }
      }
      // Dimension-only events do not advance the one-second sampling clock.
      if (_lastVideoAt == null || now - _lastVideoAt! >= 250) {
        _lastVideoFrames = frames.toInt();
        _lastVideoAt = now;
        _lastMediaCounters = Map<String, dynamic>.from(event);
      }
    }
    if (event['rttMs'] is num) {
      final sample = <String, dynamic>{
        'atMs': _diagnosticClock.elapsedMilliseconds,
      };
      for (final key in [
        'rttMs',
        'videoFps',
        'displayFps',
        'iOSDecodedFrames',
        'iOSDecodedReplaced',
        'iOSDecodeExpired',
        'iOSDecodeOverflows',
        'iOSDecodeAvgUs',
        'iOSDecodeQueueAvgUs',
        'iOSPresentWaitAvgUs',
        'iOSPresentedFrames',
        'iOSDisplayBusyTicks',
        'iOSDisplayFailures',
        'videoPackets',
        'videoFrames',
        'nackRequests',
        'lostFragments',
        'referenceDrops',
        'audioDecoded',
        'audioConcealed',
        'audioLate',
        'nackRequestsPerSecond',
        'audioConcealedPerSecond',
        'audioLatePerSecond',
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
        if (_recovering) {
          _recovering = false;
          busy = false;
          _retryAfterRun = false;
          _recoveryTimer?.cancel();
          _recoveryDeadline?.cancel();
          routeNotice = routeNotice == null
              ? 'Đã khôi phục kết nối trên mạng hiện tại.'
              : 'Đã khôi phục kết nối. $routeNotice';
        }
      case 'stopped':
        if (_recovering) {
          _acceptNative = false;
          phase = SessionPhase.reconnecting;
          _scheduleRecovery();
          break;
        }
        phase = SessionPhase.failed;
        error = event['message'] as String? ?? 'Luồng từ xa đã kết thúc.';
        unawaited(disconnect(keepError: true));
    }
    notifyListeners();
  }

  void setForeground(bool value) {
    _foreground = value;
    if (!value) {
      _recoveryTimer?.cancel();
    } else if (_recovering) {
      _scheduleRecovery();
    }
  }

  void _networkEvent(Json event) {
    if (capabilities['networkRecovery'] != true ||
        event['networkAvailable'] is! bool) {
      return;
    }
    final revision = event['networkRevision'];
    if (revision is! int || revision <= 0) return;
    if (revision < _networkRevision) return;
    if ([
      'wifi',
      'cellular',
      'mixed',
      'ethernet',
      'offline',
      'other',
    ].contains(event['networkKind'])) {
      networkDetails = {
        'networkKind': event['networkKind'],
        for (final key in ['supportsIPv4', 'supportsIPv6'])
          if (event[key] is bool) key: event[key],
      };
    }
    if (event['networkChanged'] != true) {
      _networkRevision = revision;
      _networkAvailable = event['networkAvailable'] as bool;
      return;
    }
    if (revision <= _networkRevision) return;
    _networkRevision = revision;
    _networkAvailable = event['networkAvailable'] as bool;
    if (active == null || (!_recovering && phase != SessionPhase.streaming)) {
      return;
    }
    _pathRevision++;
    if (!_recovering) {
      _recovering = true;
      _generation++;
      voiceJoined = false;
      microphone = false;
      deafened = false;
      recoveryAttempts = 0;
      final episode = ++_recoveryEpisode;
      // Recreate input/file UI ownership; never inherit held keys or drafts.
      mediaRevision++;
      _recoveryDeadline = Timer(const Duration(seconds: 45), () {
        if (_recovering && episode == _recoveryEpisode) _failRecovery();
      });
      unawaited(engine.resetInput().catchError((Object _) {}));
    }
    _acceptNative = false;
    phase = SessionPhase.reconnecting;
    busy = true;
    telemetry = {};
    _retryAfterRun = true;
    _recoveryTimer?.cancel();
    // A new network invalidates in-flight gather/check work as well.
    final direct = _direct;
    if (direct != null) unawaited(direct.cancel());
    if (_networkAvailable) _scheduleRecovery();
    notifyListeners();
  }

  void _scheduleRecovery() {
    if (_disposed ||
        !_recovering ||
        !_foreground ||
        !_networkAvailable ||
        active == null) {
      return;
    }
    _recoveryTimer?.cancel();
    _recoveryTimer = Timer(const Duration(milliseconds: 700), () {
      if (_recoveryRunning) {
        _retryAfterRun = true;
      } else {
        unawaited(_recover());
      }
    });
  }

  Future<void> _recover() async {
    if (_disposed ||
        !_recovering ||
        !_foreground ||
        !_networkAvailable ||
        active == null ||
        _recoveryRunning) {
      return;
    }
    if (recoveryAttempts >= 2) {
      _failRecovery();
      return;
    }
    _recoveryRunning = true;
    _retryAfterRun = false;
    final episode = _recoveryEpisode, id = active!['id'];
    final generation = ++_generation;
    bool current() =>
        !_disposed &&
        _recovering &&
        episode == _recoveryEpisode &&
        active?['id'] == id &&
        generation == _generation;
    try {
      _acceptNative = false;
      final oldDirect = _direct;
      _direct = null;
      final closingDirect = oldDirect?.cancel();
      try {
        await engine.disconnect().timeout(const Duration(seconds: 5));
        await closingDirect?.timeout(const Duration(seconds: 5));
      } catch (_) {
        if (current()) _failRecovery();
        return;
      }
      if (!current() || !_foreground || !_networkAvailable) return;
      recoveryAttempts++;
      notifyListeners();
      await _start(
        Map<String, dynamic>.from(active!),
        generation,
        pathRevision: _pathRevision,
      );
    } catch (e) {
      if (!current()) return;
      _acceptNative = false;
      if (e is ApiFailure && [401, 403, 404, 409].contains(e.status)) {
        _failRecovery();
      } else {
        _retryAfterRun = true;
      }
    } finally {
      if (episode == _recoveryEpisode) {
        _recoveryRunning = false;
        if (current() && _retryAfterRun) _scheduleRecovery();
      }
    }
  }

  void _failRecovery() {
    error =
        'Không khôi phục được kết nối sau khi đổi mạng. Kiểm tra mạng và kết nối lại.';
    unawaited(disconnect(keepError: true));
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
    _recoveryTimer?.cancel();
    _recoveryDeadline?.cancel();
    _recovering = false;
    _recoveryRunning = false;
    _retryAfterRun = false;
    _acceptNative = false;
    _recoveryEpisode++;
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
    final generation = _generation, id = active?['id'];
    if (id == null ||
        phase != SessionPhase.streaming ||
        capabilities['audio'] != true) {
      return;
    }
    try {
      await engine.mute(!value);
      if (generation != _generation || active?['id'] != id || _disposed) return;
      sound = value;
    } catch (e) {
      if (generation != _generation || active?['id'] != id || _disposed) return;
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
    final generation = _generation, id = active?['id'];
    final join = joined ?? voiceJoined;
    final deafen = join && (deaf ?? deafened);
    final microphoneOn = join && !deafen && (mic ?? microphone);
    try {
      await engine.voice(
        joined: join,
        microphone: microphoneOn,
        deafened: deafen,
      );
      if (_disposed || generation != _generation || active?['id'] != id) return;
      voiceJoined = join;
      microphone = microphoneOn;
      deafened = deafen;
      error = null;
    } catch (e) {
      if (_disposed || generation != _generation || active?['id'] != id) return;
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
    _recoveryTimer?.cancel();
    _recoveryDeadline?.cancel();
    _recoveryEpisode++;
    final direct = _direct;
    if (direct != null) unawaited(direct.cancel());
    api.close();
    super.dispose();
  }
}
