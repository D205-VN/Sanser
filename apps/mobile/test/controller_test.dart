import 'dart:async';
import 'dart:convert';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:sanser_mobile/src/api.dart';
import 'package:sanser_mobile/src/controller.dart';
import 'package:sanser_mobile/src/native_engine.dart';
import 'widget_test.dart' show MemorySecrets;

class TestEngine extends NativeEngine {
  final events = StreamController<Json>.broadcast(sync: true);
  Json? config;
  int disconnects = 0;
  @override
  Stream<Json> get updates => events.stream;
  @override
  Future<Json> capabilities() async => {'available': true};
  @override
  Future<void> connect(Json configuration) async {
    config = configuration;
  }

  @override
  Future<void> disconnect() async {
    disconnects++;
  }
}

void main() {
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
