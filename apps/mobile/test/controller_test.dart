import 'dart:async';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:sanser_mobile/src/api.dart';
import 'package:sanser_mobile/src/controller.dart';
import 'package:sanser_mobile/src/native_engine.dart';
import 'package:sanser_mobile/src/preferences.dart';
import 'widget_test.dart' show MemorySecrets;

class TestEngine extends NativeEngine {
  final events = StreamController<Json>.broadcast(sync: true);
  Json? config;
  int disconnects = 0;
  bool failConnect = false;
  bool direct = false;
  final operations = <String>[];
  Completer<void>? closing;
  @override
  Stream<Json> get updates => events.stream;
  @override
  Future<Json> capabilities() async => {'available': true, 'direct': direct};
  @override
  Future<Json> p2p(Json request) async {
    operations.add(request['operation'] as String);
    if (request['operation'] == 'gather') throw StateError('UDP unavailable');
    return {};
  }

  @override
  Future<void> connect(Json configuration) async {
    config = configuration;
    if (failConnect) throw StateError('relay unavailable');
  }

  @override
  Future<void> disconnect() async {
    disconnects++;
    await closing?.future;
  }
}

void main() {
  Future<MobileController> connected(
    TestEngine native, {
    String mode = 'auto',
  }) async {
    final api = SanserApi(
      MemorySecrets(),
      client: MockClient((request) async {
        final body = switch (request.url.path) {
          '/api/v2/auth/login' => {'accessToken': 'a', 'refreshToken': 'r'},
          '/api/v2/account' => {'email': 'test@example.com'},
          '/api/v2/devices' || '/api/v2/sharing' => {'items': []},
          '/api/v2/sessions' => {'id': 'session', 'state': 'accepted'},
          '/api/v2/sessions/session/native-ready' => {
            'requesterReadyAt': 1700000000,
          },
          '/api/v2/sessions/session/credentials' => {
            'wireProtocol': 'snv2',
            'peerDeviceId': 'host',
            'sessionToken': 'token',
          },
          _ => <String, dynamic>{},
        };
        return http.Response(jsonEncode(body), 200);
      }),
    );
    final c = MobileController(api, native);
    await c.initialize();
    await c.login('test@example.com', 'password', 'https://server.example');
    c.preferences = c.preferences.copyWith(networkMode: mode);
    await c.connect({'id': 'host'});
    return c;
  }

  test('Auto falls back to Relay and releases failed Direct attempt', () async {
    final native = TestEngine()..direct = true;
    final c = await connected(native);
    expect(native.config?['videoBitrateMbps'], 6);
    expect(native.config?['baselineRttMs'], 0.0);
    expect(native.operations, ['gather', 'close']);
    expect(native.config?['transport'], 'relay');
    expect(c.routeNotice, contains('Relay'));
    await c.disconnect();
    c.dispose();
    await native.events.close();
  });
  test('forced Direct failure never silently starts Relay', () async {
    final native = TestEngine()..direct = true;
    final c = await connected(native, mode: 'direct');
    expect(native.config, isNull);
    expect(c.active, isNull);
    expect(c.error, contains('Direct'));
    expect(c.phase, SessionPhase.failed);
    c.dispose();
    await native.events.close();
  });
  test('explicit Relay skips candidate gathering', () async {
    final native = TestEngine()..direct = true;
    final c = await connected(native, mode: 'relay');
    expect(native.operations, isEmpty);
    expect(native.config?['transport'], 'relay');
    await c.disconnect();
    c.dispose();
    await native.events.close();
  });
  test('network preference is validated and round trips', () {
    expect(
      MobilePreferences.fromJson({'networkMode': 'invalid'}).networkMode,
      'auto',
    );
    expect(
      MobilePreferences.fromJson(
        const MobilePreferences(networkMode: 'direct').toJson(),
      ).networkMode,
      'direct',
    );
  });

  test(
    'failed native connection clears session and keeps error visible',
    () async {
      final native = TestEngine()..failConnect = true;
      final c = await connected(native);
      expect(c.active, isNull);
      expect(c.busy, false);
      expect(c.phase, SessionPhase.failed);
      expect(c.error, contains('relay unavailable'));
      expect(native.disconnects, 1);
      c.dispose();
      await native.events.close();
    },
  );

  test(
    'remote stop and simultaneous disconnect tear down exactly once',
    () async {
      final native = TestEngine();
      final c = await connected(native);
      native.closing = Completer<void>();
      native.events.add({
        'sessionId': 'session',
        'state': 'stopped',
        'message': 'Host disconnected',
      });
      final second = c.disconnect(keepError: true);
      expect(c.active, isNull);
      expect(native.disconnects, 1);
      native.closing!.complete();
      await second;
      expect(c.phase, SessionPhase.failed);
      expect(c.error, 'Host disconnected');
      expect(c.busy, false);
      c.dispose();
      await native.events.close();
    },
  );

  test(
    'server state accepted announces ready before credentials; view-only stays read-only',
    () async {
      final paths = <String>[];
      final native = TestEngine();
      final api = SanserApi(
        MemorySecrets(),
        client: MockClient((request) async {
          paths.add(request.url.path);
          final result = switch (request.url.path) {
            '/api/v2/auth/login' => {'accessToken': 'a', 'refreshToken': 'r'},
            '/api/v2/account' => {'email': 'a@example.com'},
            '/api/v2/devices' || '/api/v2/sharing' => {'items': []},
            '/api/v2/sessions' => {
              'id': 'test-session',
              'state': 'accepted',
              'hostDeviceId': 'host',
              'accessMode': 'view-only',
            },
            '/api/v2/sessions/test-session/native-ready' => {
              'id': 'test-session',
              'state': 'accepted',
              'requesterReadyAt': 123,
            },
            '/api/v2/sessions/test-session/credentials' => {
              'wireProtocol': 'snv2',
              'sessionToken': 'token',
              'peerDeviceId': 'host',
              'accessMode': 'view-only',
            },
            _ => <String, dynamic>{},
          };
          return http.Response(jsonEncode(result), 200);
        }),
      );
      final c = MobileController(api, native);
      await c.initialize();
      await c.login('a@example.com', 'secret', 'https://server.example');
      await c.connect({'id': 'host'}, viewOnly: true);
      expect(
        paths.indexOf('/api/v2/sessions/test-session/native-ready'),
        lessThan(paths.indexOf('/api/v2/sessions/test-session/credentials')),
      );
      expect(native.config?['inputEnabled'], false);
      expect(c.phase, SessionPhase.connecting);
      native.events.add({'sessionId': 'wrong', 'state': 'frame'});
      expect(c.phase, SessionPhase.connecting);
      native.events.add({'sessionId': 'test-session', 'state': 'frame'});
      expect(c.phase, SessionPhase.streaming);
      await c.disconnect();
      expect(c.active, isNull);
      expect(paths, contains('/api/v2/sessions/test-session/disconnect'));
      native.events.add({'sessionId': 'test-session', 'state': 'frame'});
      expect(c.phase, SessionPhase.idle);
      c.dispose();
      await native.events.close();
    },
  );
}
