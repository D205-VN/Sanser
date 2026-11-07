import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/api.dart';
import 'package:sanser_mobile/src/controller.dart';
import 'package:sanser_mobile/src/mobile_input.dart';
import 'package:sanser_mobile/src/screens.dart';
import 'controller_test.dart' show TestEngine;
import 'widget_test.dart' show MemorySecrets;

class RecoveryEngine extends TestEngine {
  final configs = <Json>[];
  final order = <String>[];
  final keys = <(String, int, int, bool)>[];
  Completer<void>? inputReply, muteReply, voiceReply;
  @override
  Future<Json> capabilities() async => {
    'available': true,
    'direct': direct,
    'networkRecovery': true,
    'audio': true,
  };
  @override
  Future<void> connect(Json configuration) async {
    order.add('connect');
    configs.add(configuration);
    await super.connect(configuration);
  }

  @override
  Future<void> disconnect() async {
    order.add('disconnect');
    await super.disconnect();
  }

  @override
  Future<void> resetInput() async {
    order.add('reset');
  }

  @override
  Future<void> remoteInput(
    String id,
    int kind, {
    int x = 0,
    int y = 0,
    int code = 0,
    int delta = 0,
    bool down = false,
  }) async {
    keys.add((id, kind, code, down));
    await inputReply?.future;
  }

  @override
  Future<void> mute(bool muted) async {
    await muteReply?.future;
  }

  @override
  Future<void> voice({
    required bool joined,
    required bool microphone,
    required bool deafened,
  }) async {
    await voiceReply?.future;
  }

  void frame({int? attempt}) => events.add({
    'state': 'frame',
    'sessionId': 'session',
    'engineAttempt': attempt ?? configs.last['engineAttempt'],
  });
  void change(int revision, {bool available = true, bool changed = true}) =>
      events.add({
        'state': 'network',
        'networkRevision': revision,
        'networkAvailable': available,
        'networkChanged': changed,
      });
}

class RecoveryApi extends SanserApi {
  RecoveryApi(this.respond) : super(MemorySecrets());
  final Future<Json> Function(String) respond;
  @override
  Future<Json> request(
    String path, {
    String method = 'GET',
    Json? body,
    bool auth = true,
    bool retry = true,
  }) => respond(path.split('?').first);
}

class RecoveryFixture {
  final native = RecoveryEngine();
  final paths = <String>[];
  late MobileController c;
  int readyRequests = 0, rejectReady = 0;
  Completer<void>? credentialsGate;
  Future<void> open({bool group = false}) async {
    final api = RecoveryApi((path) async {
      paths.add(path);
      if (path.endsWith('/native-ready')) {
        readyRequests++;
        if (rejectReady != 0 && readyRequests > 1) {
          throw ApiFailure(rejectReady, 'Session revoked');
        }
      }
      if (path.endsWith('/credentials') && readyRequests > 1) {
        await credentialsGate?.future;
      }
      final body = switch (path) {
        '/api/v2/auth/login' => {'accessToken': 'a', 'refreshToken': 'r'},
        '/api/v2/account' => {'email': 'demo@example.com'},
        '/api/v2/devices' || '/api/v2/sharing' => {'items': []},
        '/api/v2/sessions' => {
          'id': 'session',
          'state': 'accepted',
          'networkMode': 'auto',
          'accessMode': group ? 'view-only' : 'control',
          if (group) 'sharingId': 'group',
          if (group) 'sharingViewerId': 'viewer',
        },
        '/api/v2/sessions/session/native-ready' => {
          'requesterReadyAt': 1700000000 + readyRequests,
        },
        '/api/v2/sessions/session/credentials' => {
          'wireProtocol': 'snv2',
          'peerDeviceId': 'host',
          'sessionToken': 'credential',
          'accessMode': group ? 'view-only' : 'control',
        },
        _ => <String, dynamic>{},
      };
      return body;
    });
    c = MobileController(api, native);
    await c.initialize();
    await c.login('demo@example.com', 'password', 'https://server.example');
    await c.connect({'id': 'host'}, viewOnly: group);
    native.frame();
    expect(c.phase, SessionPhase.streaming);
  }

  Future<void> close() async {
    if (credentialsGate?.isCompleted == false) credentialsGate!.complete();
    if (native.inputReply?.isCompleted == false) native.inputReply!.complete();
    if (native.muteReply?.isCompleted == false) native.muteReply!.complete();
    if (native.voiceReply?.isCompleted == false) native.voiceReply!.complete();
    if (native.closing?.isCompleted == false) native.closing!.complete();
    await c.disconnect();
    c.dispose();
    await native.events.close();
  }
}

void main() {
  testWidgets(
    'network baseline/duplicate notifications do not restart a healthy stream',
    (tester) async {
      final f = RecoveryFixture();
      await f.open();
      f.native.change(1, changed: false);
      f.native.change(1);
      await tester.pump(const Duration(seconds: 2));
      expect(f.native.configs.length, 1);
      expect(f.c.recovering, false);
      expect(f.c.phase, SessionPhase.streaming);
      await f.close();
    },
  );

  testWidgets(
    'handoff debounces, reuses accepted session, refreshes readiness and fences old events',
    (tester) async {
      final f = RecoveryFixture();
      await f.open();
      final oldAttempt = f.native.configs.single['engineAttempt'] as int;
      f.native.change(1, changed: false);
      f.native.change(2, available: false);
      expect(f.c.phase, SessionPhase.reconnecting);
      expect(f.c.mediaRevision, 1);
      f.native.frame(attempt: oldAttempt);
      expect(f.c.phase, SessionPhase.reconnecting);
      await tester.pump(const Duration(milliseconds: 400));
      f.native.change(3);
      await tester.pump(const Duration(milliseconds: 400));
      f.native.change(4);
      await tester.pump(const Duration(milliseconds: 699));
      expect(f.native.configs.length, 1);
      await tester.pump(const Duration(milliseconds: 1));
      await tester.pump();
      expect(f.native.configs.length, 2);
      expect(f.native.order, ['connect', 'reset', 'disconnect', 'connect']);
      expect(f.readyRequests, 2);
      expect(f.paths.where((p) => p == '/api/v2/sessions'), hasLength(1));
      expect(f.paths.where((p) => p.endsWith('/disconnect')), isEmpty);
      expect(f.native.configs.last['engineAttempt'], greaterThan(oldAttempt));
      f.native.events.add({
        'sessionId': 'session',
        'state': 'stopped',
        'engineAttempt': oldAttempt,
      });
      f.native.frame(attempt: oldAttempt);
      expect(f.c.phase, SessionPhase.reconnecting);
      f.native.frame();
      expect(f.c.phase, SessionPhase.streaming);
      expect(f.c.busy, false);
      expect(f.c.recovering, false);
      expect(f.c.routeNotice, contains('khôi phục'));
      await f.close();
    },
  );

  testWidgets(
    'offline has a bounded wait and explicit cancel prevents revival',
    (tester) async {
      final f = RecoveryFixture();
      await f.open();
      f.native.change(1, changed: false);
      f.native.change(2, available: false);
      await tester.pump(const Duration(seconds: 44));
      expect(f.native.configs.length, 1);
      expect(f.c.active, isNotNull);
      await tester.pump(const Duration(seconds: 2));
      await tester.pump();
      expect(f.c.active, isNull);
      expect(f.c.phase, SessionPhase.failed);
      expect(f.c.error, contains('Không khôi phục'));
      f.native.change(3);
      await tester.pump(const Duration(seconds: 2));
      expect(f.native.configs.length, 1);
      await f.close();
      final g = RecoveryFixture();
      await g.open();
      g.native.change(1, changed: false);
      g.native.change(2);
      await g.c.disconnect();
      await tester.pump(const Duration(seconds: 2));
      expect(g.native.configs.length, 1);
      expect(g.c.active, isNull);
      await g.close();
    },
  );

  testWidgets('a late credentials reply after cancel cannot open media', (
    tester,
  ) async {
    final f = RecoveryFixture();
    await f.open();
    f.credentialsGate = Completer<void>();
    f.native.change(1, changed: false);
    f.native.change(2);
    await tester.pump(const Duration(milliseconds: 700));
    await tester.pump();
    expect(f.readyRequests, 2);
    await f.c.disconnect();
    f.credentialsGate!.complete();
    await tester.pump();
    expect(f.native.configs.length, 1);
    expect(f.c.active, isNull);
    await f.close();
  });

  testWidgets(
    'changed path during a pending restart is serialized and reverified',
    (tester) async {
      final f = RecoveryFixture();
      await f.open();
      f.credentialsGate = Completer<void>();
      f.native.change(1, changed: false);
      f.native.change(2);
      await tester.pump(const Duration(milliseconds: 700));
      await tester.pump();
      f.native.change(3);
      await tester.pump(const Duration(milliseconds: 700));
      expect(f.readyRequests, 2);
      expect(f.native.configs.length, 1);
      f.credentialsGate!.complete();
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 700));
      await tester.pump();
      expect(f.readyRequests, 3);
      expect(f.native.configs.length, 2);
      f.native.frame();
      expect(f.c.phase, SessionPhase.streaming);
      await f.close();
    },
  );

  testWidgets(
    'forced Direct never becomes Relay and failed recovery has only two attempts',
    (tester) async {
      final f = RecoveryFixture();
      await f.open();
      f.native.direct = true;
      f.c.capabilities['direct'] = true;
      f.c.active!['networkMode'] = 'direct';
      f.native.change(1, changed: false);
      f.native.change(2);
      for (var i = 0; i < 4; i++) {
        await tester.pump(const Duration(milliseconds: 700));
        await tester.pump();
      }
      expect(f.native.configs.length, 1);
      expect(f.c.recoveryAttempts, 2);
      expect(f.native.operations.where((o) => o == 'gather'), hasLength(2));
      expect(f.c.active, isNull);
      expect(f.c.phase, SessionPhase.failed);
      await f.close();
    },
  );

  testWidgets(
    'server revocation stops immediately instead of requesting a new session',
    (tester) async {
      final f = RecoveryFixture();
      await f.open();
      f.rejectReady = 409;
      f.native.change(1, changed: false);
      f.native.change(2);
      await tester.pump(const Duration(milliseconds: 700));
      await tester.pump();
      expect(f.c.active, isNull);
      expect(f.c.recoveryAttempts, 1);
      expect(f.native.configs.length, 1);
      expect(f.paths.where((p) => p == '/api/v2/sessions'), hasLength(1));
      await f.close();
    },
  );

  testWidgets(
    'background suspends recovery and group viewer remains read-only',
    (tester) async {
      final f = RecoveryFixture();
      await f.open(group: true);
      f.c.setForeground(false);
      f.native.change(1, changed: false);
      f.native.change(2);
      await tester.pump(const Duration(seconds: 2));
      expect(f.native.configs.length, 1);
      f.c.setForeground(true);
      await tester.pump(const Duration(milliseconds: 700));
      await tester.pump();
      expect(f.native.configs.last['inputEnabled'], false);
      expect(f.native.configs.last['sharingId'], 'group');
      expect(f.native.configs.last['viewerId'], 'viewer');
      f.native.frame();
      expect(f.c.phase, SessionPhase.streaming);
      expect(f.paths.where((p) => p.endsWith('/native-started')), hasLength(2));
      await f.close();
    },
  );

  testWidgets(
    'pending input and audio/voice replies cannot leak into the recovered generation',
    (tester) async {
      final f = RecoveryFixture();
      await f.open();
      final epoch = f.c.mediaRevision;
      final input = MobileInput(
        f.native,
        'session',
        enabled: true,
        current: () =>
            f.c.mediaRevision == epoch && f.c.phase == SessionPhase.streaming,
      );
      f.native.inputReply = Completer<void>();
      final press = input.hold(225, true, 'hardware');
      await tester.pump();
      final queued = input.hold(4, true, 'hardware');
      f.native.muteReply = Completer<void>();
      final mute = f.c.setSound(false);
      f.native.voiceReply = Completer<void>();
      final voice = f.c.setVoice(joined: true, mic: true);
      f.native.change(1, changed: false);
      f.native.change(2);
      f.native.inputReply!.complete();
      f.native.muteReply!.complete();
      f.native.voiceReply!.complete();
      await press;
      await queued;
      await mute;
      await voice;
      expect(f.native.keys, [('session', 2, 225, true)]);
      expect(f.c.sound, true);
      expect(f.c.microphone, false);
      expect(f.c.voiceJoined, false);
      await tester.pump(const Duration(milliseconds: 700));
      await tester.pump();
      f.native.frame();
      input.dispose();
      await input.drain();
      expect(f.native.keys, [('session', 2, 225, true)]);
      expect(f.native.configs.last['audioMuted'], false);
      await f.close();
    },
  );

  testWidgets(
    'recovery UI exposes waiting state and Cancel without remote controls',
    (tester) async {
      final f = RecoveryFixture();
      await f.open();
      await tester.pumpWidget(MaterialApp(home: SanserHome(controller: f.c)));
      f.native.change(1, changed: false);
      f.native.change(2, available: false);
      await tester.pump();
      expect(find.textContaining('Mất mạng'), findsOneWidget);
      expect(find.widgetWithText(TextButton, 'Hủy kết nối'), findsOneWidget);
      expect(find.byType(RemoteSession), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
      await f.close();
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );
  testWidgets(
    'Auto retries Direct with fresh candidates and may fall back to Relay without changing mute',
    (tester) async {
      final f = RecoveryFixture();
      await f.open();
      f.native.direct = true;
      f.c.capabilities['direct'] = true;
      f.c.sound = false;
      f.native.change(1, changed: false);
      f.native.change(2);
      await tester.pump(const Duration(milliseconds: 700));
      await tester.pump();
      expect(f.native.operations, containsAllInOrder(['gather', 'close']));
      expect(f.native.configs.length, 2);
      expect(f.native.configs.last['transport'], 'relay');
      expect(f.native.configs.last['audioMuted'], true);
      f.native.frame();
      expect(f.c.routeNotice, contains('Relay'));
      await f.close();
    },
  );

  testWidgets(
    'native startup failure consumes remaining budget, but stalled cleanup never opens another engine',
    (tester) async {
      final f = RecoveryFixture();
      await f.open();
      f.native.change(1, changed: false);
      f.native.change(2);
      await tester.pump(const Duration(milliseconds: 700));
      await tester.pump();
      f.native.events.add({
        'sessionId': 'session',
        'state': 'stopped',
        'engineAttempt': f.native.configs.last['engineAttempt'],
      });
      await tester.pump(const Duration(milliseconds: 700));
      await tester.pump();
      expect(f.native.configs.length, 3);
      expect(f.c.recoveryAttempts, 2);
      f.native.frame();
      expect(f.c.phase, SessionPhase.streaming);
      await f.close();
      final g = RecoveryFixture();
      await g.open();
      g.native.closing = Completer<void>();
      g.native.change(1, changed: false);
      g.native.change(2);
      await tester.pump(const Duration(milliseconds: 700));
      await tester.pump(const Duration(seconds: 6));
      await tester.pump();
      expect(g.c.active, isNull);
      expect(g.c.phase, SessionPhase.failed);
      expect(g.native.configs.length, 1);
      await g.close();
    },
  );
  testWidgets(
    'local controller motion never changes session telemetry or rebuilds the workspace',
    (tester) async {
      final f = RecoveryFixture();
      await f.open();
      var notices = 0;
      f.c.addListener(() => notices++);
      final attempt = f.native.configs.single['engineAttempt'];
      for (var i = 0; i < 100; i++) {
        f.native.events.add({
          'state': 'controller',
          'sessionId': 'session',
          'engineAttempt': attempt,
          'lx': i / 100,
          'buttons': 0,
        });
      }
      expect(notices, 0);
      expect(f.c.telemetry.containsKey('lx'), false);
      expect(f.c.phase, SessionPhase.streaming);
      await f.close();
    },
  );
}
