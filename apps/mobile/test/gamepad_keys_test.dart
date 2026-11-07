import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/api.dart';
import 'package:sanser_mobile/src/gamepad_keys.dart';
import 'package:sanser_mobile/src/mobile_input.dart';
import 'package:sanser_mobile/src/native_engine.dart';
import 'session_actions_test.dart'
    show ActionEngine, active, showControls, finish;

Json state({
  int buttons = 0,
  double lx = 0,
  double ly = 0,
  double rx = 0,
  double ry = 0,
  double lt = 0,
  double rt = 0,
  bool connected = true,
  bool sample = true,
}) => {
  'buttons': buttons,
  'lx': lx,
  'ly': ly,
  'rx': rx,
  'ry': ry,
  'lt': lt,
  'rt': rt,
  'connected': connected,
  'sample': sample,
};

class PadEngine extends ActionEngine {
  final events = StreamController<Json>.broadcast(sync: true);
  final captures = <(String, int, int, bool)>[];
  Completer<void>? reply;
  @override
  Stream<Json> get updates => events.stream;
  @override
  Future<void> controllerCapture(
    String id,
    int attempt,
    int capture,
    bool enabled,
  ) async {
    captures.add((id, attempt, capture, enabled));
    if (enabled) await reply?.future;
  }

  void snapshot(
    Json value, {
    int? capture,
    int? attempt,
    String id = 'session-a',
  }) {
    final last = captures.last;
    events.add({
      ...value,
      'state': 'controller',
      'sessionId': id,
      'engineAttempt': attempt ?? last.$2,
      'captureId': capture ?? last.$3,
    });
  }
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  test(
    'no held-before-capture input; neutral sample arms once; availability is not a sample',
    () async {
      final e = PadEngine(),
          input = MobileInput(e, 's', enabled: true),
          m = GamepadKeyMapper(input);
      m.update(state(sample: false));
      expect(m.armed, false);
      m.update(state(buttons: 1, lx: 1));
      await input.drain();
      expect(e.input, isEmpty);
      m.update(state());
      expect(m.armed, true);
      m.update(state(buttons: 1));
      await input.drain();
      expect(e.input, [('s', 2, 44, true)]);
      m.update(state(connected: false));
      await input.drain();
      expect(e.input.last, ('s', 2, 44, false));
      expect(m.armed, false);
      m.update(state(lx: 1));
      await input.drain();
      expect(e.input.length, 2);
      m.dispose();
      input.dispose();
      await input.drain();
      await e.events.close();
    },
  );
  test(
    'stick hysteresis suppresses noise and direction reversal releases before pressing',
    () async {
      final e = PadEngine(),
          input = MobileInput(e, 's', enabled: true),
          m = GamepadKeyMapper(input);
      m.update(state());
      m.update(state(lx: .36));
      m.update(state(lx: .30));
      m.update(state(lx: .21));
      await input.drain();
      expect(e.input, [('s', 2, 7, true)]);
      m.update(state(lx: -.8));
      m.update(state(lx: -.30));
      m.update(state(lx: -.19));
      await input.drain();
      expect(e.input, [
        ('s', 2, 7, true),
        ('s', 2, 7, false),
        ('s', 2, 4, true),
        ('s', 2, 4, false),
      ]);
      m.dispose();
      input.dispose();
      await input.drain();
      await e.events.close();
    },
  );
  test(
    'button snapshots keep edges, suppress duplicates and map the documented HID profile',
    () async {
      final e = PadEngine(),
          input = MobileInput(e, 's', enabled: true),
          m = GamepadKeyMapper(input);
      m.update(state());
      const codes = [44, 41, 8, 21, 20, 9, 40, 43, 225, 224, 82, 81, 80, 79];
      for (var i = 0; i < codes.length; i++) {
        m.update(state(buttons: 1 << i));
        m.update(state(buttons: 1 << i));
        m.update(state());
      }
      await input.drain();
      expect(e.input, [
        for (final code in codes) ...[
          ('s', 2, code, true),
          ('s', 2, code, false),
        ],
      ]);
      m.dispose();
      input.dispose();
      await input.drain();
      await e.events.close();
    },
  );
  test(
    'trigger and stick ownership never releases touch or another physical control',
    () async {
      final e = PadEngine(),
          input = MobileInput(e, 's', enabled: true),
          m = GamepadKeyMapper(input);
      await input.hold(225, true, 'touch');
      m.update(state());
      m.update(state(lt: .6, buttons: 1 << 8));
      m.update(state(lt: .4));
      m.update(state(lt: .34));
      await input.drain();
      expect(e.input, [('s', 2, 225, true)]);
      m.update(state(rx: .8, buttons: 1 << 13));
      m.update(state(rx: .8));
      await input.drain();
      expect(e.input.last, ('s', 2, 79, true));
      expect(e.input.length, 2);
      m.release();
      await input.drain();
      expect(e.input.last, ('s', 2, 79, false));
      await input.hold(225, false, 'touch');
      expect(e.input.last, ('s', 2, 225, false));
      m.dispose();
      input.dispose();
      await input.drain();
      await e.events.close();
    },
  );
  test(
    'malformed snapshots fail neutral; disposed/read-only mappers cannot inject',
    () async {
      final e = PadEngine(),
          input = MobileInput(e, 's', enabled: true),
          m = GamepadKeyMapper(input);
      for (final bad in [
        <String, dynamic>{'lx': double.nan},
        {'ry': double.infinity},
        {'buttons': -1},
        {'buttons': 0x4000},
        {'buttons': '1'},
        {'lt': -1},
        {'rt': 2},
        {'connected': null},
        {'sample': null},
      ]) {
        m.update(state());
        m.update(state(buttons: 1));
        expect(m.update({...state(), ...bad}), false);
      }
      await input.drain();
      expect(e.input.where((v) => v.$4), hasLength(9));
      expect(e.input.where((v) => !v.$4), hasLength(9));
      m.dispose();
      expect(m.update(state(buttons: 1)), false);
      input.dispose();
      await input.drain();
      e.input.clear();
      final viewer = MobileInput(e, 'view', enabled: false);
      final vm = GamepadKeyMapper(viewer);
      vm.update(state());
      vm.update(state(buttons: 1));
      vm.dispose();
      viewer.dispose();
      await viewer.drain();
      expect(e.input, isEmpty);
      await e.events.close();
    },
  );
  test(
    'capture binds events to session, native attempt and activation; blur releases only owned keys',
    () async {
      final e = PadEngine(), input = MobileInput(e, 'session-a', enabled: true);
      var ready = true, attempt = 4;
      final capture = GamepadKeyCapture(
        e,
        input,
        current: () => ready,
        attempt: () => attempt,
      );
      capture.setCapture(true);
      final first = e.captures.last;
      e.snapshot(state());
      e.snapshot(state(buttons: 1));
      await input.drain();
      expect(e.input.last, ('session-a', 2, 44, true));
      e.snapshot(state(buttons: 2), id: 'wrong');
      e.snapshot(state(buttons: 2), attempt: 3);
      await input.drain();
      expect(e.input.length, 1);
      capture.setCapture(false);
      await input.drain();
      expect(e.input.last, ('session-a', 2, 44, false));
      capture.setCapture(true);
      expect(e.captures.last.$3, isNot(first.$3));
      e.snapshot(state(buttons: 2), capture: first.$3);
      await input.drain();
      expect(e.input.length, 2);
      e.snapshot(state());
      e.snapshot(state(buttons: 2));
      await input.drain();
      expect(e.input.last, ('session-a', 2, 41, true));
      ready = false;
      e.snapshot(state(buttons: 4));
      await input.drain();
      expect(e.input.last, ('session-a', 2, 41, false));
      expect(e.captures.last.$4, false);
      ready = true;
      attempt = 5;
      capture.setCapture(true);
      e.snapshot(state(buttons: 1), attempt: 4);
      await input.drain();
      expect(e.input.length, 4);
      capture.dispose();
      input.dispose();
      await input.drain();
      await e.events.close();
    },
  );
  test(
    'Start+Select and native focus pause release capture without forwarding the chord',
    () async {
      final e = PadEngine(), input = MobileInput(e, 'session-a', enabled: true);
      var released = 0;
      final capture = GamepadKeyCapture(
        e,
        input,
        current: () => true,
        attempt: () => 1,
        onReleased: () => released++,
      );
      capture.setCapture(true);
      e.snapshot(state());
      e.snapshot(state(buttons: 1 << 6));
      e.snapshot(state(buttons: 0xc0));
      await input.drain();
      expect(released, 1);
      expect(e.input, [
        ('session-a', 2, 40, true),
        ('session-a', 2, 40, false),
      ]);
      expect(e.captures.last.$4, false);
      capture.setCapture(true);
      e.snapshot(state());
      e.snapshot(state(buttons: 1));
      e.snapshot({...state(), 'paused': true});
      await input.drain();
      expect(released, 2);
      expect(e.input.last, ('session-a', 2, 44, false));
      capture.dispose();
      input.dispose();
      await input.drain();
      await e.events.close();
    },
  );
  test(
    'axis changes do not notify the UI every frame and a late enable reply cannot revive capture',
    () async {
      final e = PadEngine(), input = MobileInput(e, 'session-a', enabled: true);
      final capture = GamepadKeyCapture(
        e,
        input,
        current: () => true,
        attempt: () => 1,
      );
      var notices = 0;
      capture.addListener(() => notices++);
      e.reply = Completer<void>();
      capture.setCapture(true);
      e.snapshot(state());
      final count = notices;
      for (var i = 0; i < 100; i++) {
        e.snapshot(state(lx: .05 * (i % 3)));
      }
      expect(notices, count);
      await input.drain();
      expect(e.input, isEmpty);
      capture.setCapture(false);
      e.reply!.complete();
      await Future<void>.delayed(Duration.zero);
      e.snapshot(state(buttons: 1));
      await input.drain();
      expect(e.input, isEmpty);
      expect(e.captures.last.$4, false);
      capture.dispose();
      input.dispose();
      await input.drain();
      await e.events.close();
    },
  );
  test(
    'native capture rejection and stream error cannot leave held keys',
    () async {
      final e = PadEngine(), input = MobileInput(e, 'session-a', enabled: true);
      final capture = GamepadKeyCapture(
        e,
        input,
        current: () => true,
        attempt: () => 1,
      );
      e.reply = Completer<void>();
      capture.setCapture(true);
      e.reply!.completeError(StateError('rejected'));
      await Future<void>.delayed(Duration.zero);
      expect(capture.error, isNotNull);
      expect(e.captures.last.$4, false);
      e.reply = null;
      capture.setCapture(true);
      e.snapshot(state());
      e.snapshot(state(buttons: 1));
      e.events.addError(StateError('gone'));
      await input.drain();
      expect(e.input.last, ('session-a', 2, 44, false));
      expect(capture.error, isNotNull);
      capture.dispose();
      input.dispose();
      await input.drain();
      await e.events.close();
    },
  );
  test(
    'native status stream has one platform subscription shared by session and controller capture',
    () async {
      final binding = TestWidgetsFlutterBinding.ensureInitialized(),
          calls = <String>[];
      final channel = MethodChannel('com.sanser.mobile/events');
      binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
        call,
      ) async {
        calls.add(call.method);
        return null;
      });
      final engine = NativeEngine(), a = <Json>[], b = <Json>[];
      final session = engine.updates.listen(a.add),
          pad = engine.updates.listen(b.add);
      await Future<void>.delayed(Duration.zero);
      binding.channelBuffers.push(
        'com.sanser.mobile/events',
        const StandardMethodCodec().encodeSuccessEnvelope({'state': 'frame'}),
        (_) {},
      );
      await Future<void>.delayed(Duration.zero);
      expect(a.length, 1);
      expect(b.length, 1);
      expect(calls, ['listen']);
      await pad.cancel();
      expect(calls, ['listen']);
      await session.cancel();
      await Future<void>.delayed(Duration.zero);
      expect(calls, ['listen', 'cancel']);
      binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, null);
    },
  );
  testWidgets(
    'controller-key mode is opt-in, capability gated and suspends for a local editor',
    (tester) async {
      final e = PadEngine(),
          c = active(e)..capabilities['controllerKeys'] = true;
      await showControls(tester, c);
      expect(e.captures, isEmpty);
      final action = find.widgetWithText(OutlinedButton, 'Tay cầm → phím');
      await tester.ensureVisible(action);
      await tester.pumpAndSettle();
      await tester.tap(action);
      await tester.pump();
      await tester.pump();
      expect(e.captures.last.$4, true);
      e.snapshot(state());
      e.snapshot(state(buttons: 1));
      await tester.pump();
      expect(e.input.where((v) => v.$2 == 2).last, ('session-a', 2, 44, true));
      await tester.ensureVisible(find.text('Bàn phím'));
      await tester.tap(find.text('Bàn phím'));
      await tester.pumpAndSettle();
      expect(e.captures.last.$4, false);
      final count = e.input.length;
      e.snapshot(state(buttons: 2));
      await tester.enterText(find.byType(TextField), 'local draft');
      await tester.pump();
      expect(e.input.length, count);
      Navigator.of(tester.element(find.byType(TextField))).pop();
      await tester.pumpAndSettle();
      await finish(tester, c);
      await e.events.close();
      final viewer = active(ActionEngine(), viewOnly: true)
        ..capabilities['controllerKeys'] = true;
      await showControls(tester, viewer);
      expect(find.text('Tay cầm → phím'), findsNothing);
      await finish(tester, viewer);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );
  test(
    'closed native status stream releases keys and refuses a blind reactivation',
    () async {
      final e = PadEngine(), input = MobileInput(e, 'session-a', enabled: true);
      final capture = GamepadKeyCapture(
        e,
        input,
        current: () => true,
        attempt: () => 1,
      );
      capture.setCapture(true);
      e.snapshot(state());
      e.snapshot(state(buttons: 1));
      await input.drain();
      await e.events.close();
      await input.drain();
      expect(e.input.last, ('session-a', 2, 44, false));
      expect(capture.error, isNotNull);
      final count = e.captures.length;
      capture.setCapture(true);
      expect(e.captures.length, count);
      capture.dispose();
      input.dispose();
      await input.drain();
    },
  );
}
