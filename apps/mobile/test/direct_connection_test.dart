import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:sanser_mobile/src/api.dart';
import 'package:sanser_mobile/src/direct_connection.dart';
import 'package:sanser_mobile/src/native_engine.dart';
import 'widget_test.dart' show MemorySecrets;

class ProbeEngine extends NativeEngine {
  final operations = <String>[];
  Completer<Json>? gather;
  bool stunSucceeded = true, rejectCheck = false;
  @override
  Future<Json> p2p(Json request) async {
    final operation = request['operation'] as String;
    operations.add(operation);
    if (operation == 'check' && rejectCheck) {
      throw StateError('UDP check failed');
    }
    return switch (operation) {
      'gather' =>
        gather == null
            ? {
                'stunSucceeded': stunSucceeded,
                'candidates': [
                  {'id': 'local'},
                ],
              }
            : await gather!.future,
      'check' => {
        'transport': 'direct',
        'probe': {'medianMs': 8},
      },
      'activate' => {
        'endpoint': '127.0.0.1:40001',
        'localPort': 40002,
        'transport': 'direct',
      },
      _ => <String, dynamic>{},
    };
  }
}

class SignalSocket extends Stream<dynamic> implements WebSocket {
  final incoming = StreamController<dynamic>();
  final sent = <Json>[];
  bool failClose = false;
  int closes = 0;
  @override
  String get protocol => 'sanser-v2';
  @override
  int get readyState => WebSocket.open;
  @override
  void add(dynamic data) =>
      sent.add(Map<String, dynamic>.from(jsonDecode(data as String) as Map));
  @override
  StreamSubscription<dynamic> listen(
    void Function(dynamic)? onData, {
    Function? onError,
    void Function()? onDone,
    bool? cancelOnError,
  }) => incoming.stream.listen(
    onData,
    onError: onError,
    onDone: onDone,
    cancelOnError: cancelOnError,
  );
  @override
  Future<void> close([int? code, String? reason]) async {
    closes++;
    unawaited(incoming.close());
    if (failClose) throw StateError('reset without close handshake');
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

void main() {
  SanserApi api() => SanserApi(
    MemorySecrets(),
    client: MockClient(
      (request) async => http.Response(jsonEncode({'iceServers': []}), 200),
    ),
  )..useServer('https://server.example');
  Future<Json> connect(DirectConnection route) => route.connect(
    api: api(),
    sessionId: 'session',
    deviceId: 'mobile',
    peerDeviceId: 'desktop',
    sessionToken: 'credential',
    accessToken: 'secret',
    readyAt: 1700000000,
  );
  Future<void> until(bool Function() condition) async {
    for (var i = 0; i < 1000 && !condition(); i++) {
      await Future<void>.delayed(const Duration(milliseconds: 1));
    }
    expect(condition(), isTrue);
  }

  test(
    'requires matching peer/generation and receipt before authenticating UDP',
    () async {
      final engine = ProbeEngine(), socket = SignalSocket()..failClose = true;
      final route = DirectConnection(
        engine,
        open: (url, protocols) async {
          expect(url.scheme, 'wss');
          expect(protocols, ['sanser-v2', 'bearer.secret']);
          return socket;
        },
      );
      final result = connect(route);
      await until(() => socket.sent.isNotEmpty);
      void signal(
        String type, {
        int generation = 1700000000,
        String sender = 'desktop',
      }) => socket.incoming.add(
        jsonEncode({
          'sessionId': 'session',
          'senderDeviceId': sender,
          'type': type,
          'payload': {
            'generation': generation,
            'candidates': [
              {'id': 'remote'},
            ],
          },
        }),
      );
      signal('p2p.candidates', generation: 1699999999);
      signal('p2p.candidatesAck', sender: 'another-device');
      signal('p2p.candidates');
      await until(
        () => socket.sent.any((m) => m['type'] == 'p2p.candidatesAck'),
      );
      expect(engine.operations, ['gather']);
      signal('p2p.candidatesAck');
      final selected = await result;
      expect(selected['transport'], 'direct');
      expect(selected['localPort'], 40002);
      expect(engine.operations, ['gather', 'check', 'activate']);
      expect(route.active, isTrue);
      await route.cancel();
      expect(engine.operations.last, 'close');
    },
  );
  test(
    'cancel during gather closes a route even if native work starts late',
    () async {
      final engine = ProbeEngine()..gather = Completer<Json>();
      final route = DirectConnection(
        engine,
        open: (_, _) async => throw StateError('must not open'),
      );
      final result = connect(route);
      final failure = expectLater(result, throwsA(isA<ApiFailure>()));
      await until(() => engine.operations.contains('gather'));
      await route.cancel();
      engine.gather!.complete({
        'candidates': [
          {'id': 'late'},
        ],
      });
      await failure;
      expect(engine.operations, ['gather', 'close', 'close']);
    },
  );
  test('late signaling socket is closed after cancel', () async {
    final engine = ProbeEngine();
    final opened = Completer<WebSocket>();
    var opening = false;
    final route = DirectConnection(
      engine,
      open: (_, _) {
        opening = true;
        return opened.future;
      },
    );
    final result = connect(route);
    final failure = expectLater(result, throwsA(isA<ApiFailure>()));
    await until(() => opening);
    await route.cancel();
    final socket = SignalSocket();
    opened.complete(socket);
    await failure;
    expect(socket.closes, 1);
    expect(engine.operations, ['gather', 'close']);
  });
  test('signaling failure is not reported as NAT or UDP failure', () async {
    final engine = ProbeEngine(), socket = SignalSocket();
    final route = DirectConnection(engine, open: (_, _) async => socket);
    final failure = expectLater(connect(route), throwsA(isA<ApiFailure>()));
    await until(() => socket.sent.isNotEmpty);
    await socket.close();
    await failure;
    expect(route.diagnostics['stage'], 'signaling');
    expect(route.failureReason, contains('Chưa nhận được địa chỉ Direct'));
    await route.cancel();
  });
  for (final stun in [true, false]) {
    test('UDP failure distinguishes STUN result: $stun', () async {
      final engine = ProbeEngine()
        ..stunSucceeded = stun
        ..rejectCheck = true;
      final socket = SignalSocket();
      final route = DirectConnection(engine, open: (_, _) async => socket);
      final failure = expectLater(connect(route), throwsStateError);
      await until(() => socket.sent.isNotEmpty);
      for (final type in ['p2p.candidates', 'p2p.candidatesAck']) {
        socket.incoming.add(
          jsonEncode({
            'sessionId': 'session',
            'senderDeviceId': 'desktop',
            'type': type,
            'payload': {
              'generation': 1700000000,
              'candidates': [
                {'id': 'remote'},
              ],
            },
          }),
        );
      }
      await failure;
      expect(route.diagnostics, containsPair('stage', 'check'));
      expect(route.diagnostics, containsPair('stunSucceeded', stun));
      expect(
        route.failureReason,
        contains(stun ? 'Đã trao đổi địa chỉ' : 'STUN'),
      );
      await route.cancel();
    });
  }
}
