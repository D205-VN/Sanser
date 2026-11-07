import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/gamepad_keys.dart';
import 'package:sanser_mobile/src/mobile_input.dart';
import 'package:sanser_mobile/src/native_engine.dart';
import 'gamepad_keys_test.dart' show PadEngine, state;
import 'session_actions_test.dart' show active, showControls, finish;

class AnalogEngine extends PadEngine {
  final analogCaptures = <(String, int, int, bool)>[];
  Completer<void>? resetReply;
  Completer<void>? analogReply;
  @override
  Future<void> analogControllerCapture(
    String id,
    int attempt,
    int capture,
    bool enabled,
  ) async {
    analogCaptures.add((id, attempt, capture, enabled));
    if (enabled) await analogReply?.future;
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
    await super.remoteInput(
      id,
      kind,
      x: x,
      y: y,
      code: code,
      delta: delta,
      down: down,
    );
    if (kind == 4) await resetReply?.future;
  }

  void analogStatus({
    bool armed = false,
    bool connected = true,
    int? capture,
    int? attempt,
    bool paused = false,
  }) {
    final last = analogCaptures.last;
    events.add({
      'state': 'controller',
      'sessionId': last.$1,
      'engineAttempt': attempt ?? last.$2,
      'captureId': capture ?? last.$3,
      'analog': true,
      'analogArmed': armed,
      'connected': connected,
      'paused': paused,
    });
  }
}

Future<void> settle(MobileInput input) async {
  await input.drain();
  await Future<void>.delayed(Duration.zero);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test(
    'analog bridge requests native capture, never forwards analog samples through Flutter input',
    () async {
      MethodCall? last;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(NativeEngine.channel, (call) async {
            last = call;
            return null;
          });
      await NativeEngine().analogControllerCapture('s', 2, 7, true);
      expect(last!.method, 'controllerCapture');
      expect(last!.arguments, {
        'sessionId': 's',
        'engineAttempt': 2,
        'captureId': 7,
        'enabled': true,
        'mode': 'analog',
      });
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(NativeEngine.channel, null);
    },
  );
  test(
    'analog status does not produce mapped keys or heartbeat method calls',
    () async {
      final e = AnalogEngine(), input = MobileInput(e, 's', enabled: true);
      final c = GamepadKeyCapture(
        e,
        input,
        current: () => true,
        attempt: () => 1,
      )..setAnalog(true);
      c.setCapture(true);
      await settle(input);
      expect(e.analogCaptures.single.$4, true);
      expect(e.captures, isEmpty);
      expect(e.input.map((e) => e.$2), [4]);
      var updates = 0;
      c.addListener(() => updates++);
      e.analogStatus();
      e.analogStatus(armed: true);
      final before = updates;
      for (var i = 0; i < 100; i++) {
        e.analogStatus(armed: true);
      }
      await settle(input);
      expect(c.connected && c.armed, true);
      expect(updates, before);
      expect(e.analogCaptures.length, 1);
      expect(e.input.map((e) => e.$2), [4]);
      c.dispose();
      input.dispose();
      await input.drain();
      await e.events.close();
    },
  );
  test(
    'capture disable before a delayed reset never starts native analog',
    () async {
      final e = AnalogEngine()..resetReply = Completer<void>(),
          input = MobileInput(e, 's', enabled: true);
      final c = GamepadKeyCapture(
        e,
        input,
        current: () => true,
        attempt: () => 1,
      )..setAnalog(true);
      c.setCapture(true);
      await Future<void>.delayed(Duration.zero);
      c.setCapture(false);
      e.resetReply!.complete();
      await settle(input);
      expect(e.analogCaptures.where((e) => e.$4), isEmpty);
      c.dispose();
      input.dispose();
      await input.drain();
      await e.events.close();
    },
  );
  test(
    'old analog events/replies cannot arm or cancel a replacement keyboard capture',
    () async {
      final e = AnalogEngine()..analogReply = Completer<void>(),
          input = MobileInput(e, 's', enabled: true);
      var attempt = 1;
      final c = GamepadKeyCapture(
        e,
        input,
        current: () => true,
        attempt: () => attempt,
      )..setAnalog(true);
      c.setCapture(true);
      await settle(input);
      final old = e.analogCaptures.first;
      c.setAnalog(false);
      c.setCapture(true);
      e.snapshot(state(), id: 's');
      expect(c.armed, true);
      e.analogStatus(armed: true, capture: old.$3, paused: true);
      expect(c.armed, true);
      e.analogReply!.completeError(StateError('old enable failed'));
      await Future<void>.delayed(Duration.zero);
      expect(c.armed, true);
      expect(e.analogCaptures.last.$4, false);
      attempt++;
      e.snapshot(state(), id: 's');
      expect(e.captures.last.$4, false);
      c.dispose();
      input.dispose();
      await input.drain();
      await e.events.close();
    },
  );
  test(
    'paused/closed native analog releases capture and refuses blind restart',
    () async {
      final e = AnalogEngine(), input = MobileInput(e, 's', enabled: true);
      var releases = 0;
      final c = GamepadKeyCapture(
        e,
        input,
        current: () => true,
        attempt: () => 1,
        onReleased: () => releases++,
      )..setAnalog(true);
      c.setCapture(true);
      await settle(input);
      e.analogStatus(armed: true);
      e.analogStatus(paused: true);
      expect(c.connected || c.armed, false);
      expect(releases, 1);
      expect(e.analogCaptures.last.$4, false);
      c.setCapture(true);
      await settle(input);
      await e.events.close();
      final before = e.analogCaptures.length;
      c.setCapture(true);
      await settle(input);
      expect(e.analogCaptures.length, before);
      expect(c.error, contains('đã đóng'));
      c.dispose();
      input.dispose();
      await input.drain();
    },
  );
  testWidgets(
    'analog UI is gated by host capability, exclusive with keys and pauses for editor',
    (tester) async {
      final e = AnalogEngine(),
          c = active(e)
            ..capabilities.addAll({
              'controllerKeys': true,
              'controllerAnalog': true,
            });
      await showControls(tester, c);
      var action = find.widgetWithText(OutlinedButton, 'Tay cầm analog');
      expect(tester.widget<OutlinedButton>(action).onPressed, isNull);
      c.telemetry['gamepad'] = true;
      c.notifyListeners();
      await tester.pump();
      await tester.ensureVisible(action);
      await tester.pumpAndSettle();
      await tester.tap(action);
      await tester.pump();
      await tester.pump();
      expect(e.analogCaptures.last.$4, true);
      e.analogStatus(armed: true);
      await tester.pump();
      expect(find.textContaining('Analog ·'), findsOneWidget);
      c.telemetry['gamepad'] = false;
      c.notifyListeners();
      await tester.pump();
      await tester.pump();
      expect(e.analogCaptures.last.$4, false);
      final captures = e.analogCaptures.length;
      c.telemetry['gamepad'] = true;
      c.notifyListeners();
      await tester.pump();
      expect(e.analogCaptures.length, captures);
      final keys = find.widgetWithText(OutlinedButton, 'Tay cầm → phím');
      await tester.ensureVisible(keys);
      await tester.tap(keys);
      await tester.pump();
      await tester.pump();
      expect(e.analogCaptures.last.$4, false);
      expect(e.captures.last.$4, true);
      await tester.ensureVisible(action);
      await tester.tap(action);
      await tester.pump();
      await tester.pump();
      expect(e.captures.last.$4, false);
      await tester.ensureVisible(find.text('Bàn phím'));
      await tester.tap(find.text('Bàn phím'));
      await tester.pumpAndSettle();
      expect(e.analogCaptures.last.$4, false);
      await finish(tester, c);
      await e.events.close();
      final viewerEngine = AnalogEngine();
      final viewer = active(viewerEngine, viewOnly: true)
        ..capabilities['controllerAnalog'] = true;
      await showControls(tester, viewer);
      expect(find.text('Tay cầm analog'), findsNothing);
      await finish(tester, viewer);
      await viewerEngine.events.close();
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );
}
