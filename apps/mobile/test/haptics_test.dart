import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/haptics.dart';
import 'package:sanser_mobile/src/mobile_input.dart';
import 'package:sanser_mobile/src/preferences.dart';
import 'package:sanser_mobile/src/touch_surface.dart';
import 'mobile_input_test.dart' show InputEngine;

void main() {
  final binding = TestWidgetsFlutterBinding.ensureInitialized();
  final calls = <MethodCall>[];
  Future<Object?> Function(MethodCall)? reply;
  setUp(() {
    calls.clear();
    reply = null;
    binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      (call) async {
        if (call.method != 'HapticFeedback.vibrate') return null;
        calls.add(call);
        return await reply?.call(call);
      },
    );
  });
  tearDown(() {
    binding.defaultBinaryMessenger.setMockMethodCallHandler(
      SystemChannels.platform,
      null,
    );
    debugDefaultTargetPlatformOverride = null;
    binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
  });
  Future<void> flush() => Future<void>.delayed(Duration.zero);

  test(
    'haptics preference migrates off, round-trips and preserves other settings',
    () {
      expect(const MobilePreferences().haptics, false);
      for (final value in [null, 'true', 1, false]) {
        expect(MobilePreferences.fromJson({'haptics': value}).haptics, false);
      }
      final p = const MobilePreferences(
        networkMode: 'direct',
        pointerMode: 'touchpad',
      ).copyWith(haptics: true, videoBitrateMbps: 8);
      final restored = MobilePreferences.fromJson(p.toJson());
      expect(restored.haptics, true);
      expect(restored.networkMode, 'direct');
      expect(restored.pointerMode, 'touchpad');
      expect(restored.videoBitrateMbps, 8);
      expect(restored.copyWith(showLatency: false).haptics, true);
      expect(restored.copyWith(haptics: false).haptics, false);
    },
  );

  test(
    'only opted-in ready foreground mobile sessions emit local feedback',
    () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      var enabled = false, ready = true;
      var now = Duration.zero;
      final h = MobileHaptics(
        enabled: () => enabled,
        current: () => ready,
        elapsed: () => now,
      );
      h.tap();
      await flush();
      expect(calls, isEmpty);
      enabled = true;
      ready = false;
      h.tap();
      await flush();
      expect(calls, isEmpty);
      ready = true;
      binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      h.hold();
      await flush();
      expect(calls, isEmpty);
      binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      // Resume itself does not replay any attempted feedback.
      await flush();
      expect(calls, isEmpty);
      h.tap();
      await flush();
      expect(calls.single.method, 'HapticFeedback.vibrate');
      expect(calls.single.arguments, 'HapticFeedbackType.selectionClick');
      now += const Duration(milliseconds: 100);
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      h.hold();
      await flush();
      expect(calls.last.arguments, 'HapticFeedbackType.lightImpact');
      now += const Duration(milliseconds: 100);
      debugDefaultTargetPlatformOverride = TargetPlatform.linux;
      h.tap();
      await flush();
      expect(calls.length, 2);
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      h.dispose();
      h.tap();
      await flush();
      expect(calls.length, 2);
    },
  );

  test(
    'pending or rapid feedback is dropped, never queued or replayed',
    () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      final pending = Completer<Object?>();
      var now = Duration.zero;
      final h = MobileHaptics(
        enabled: () => true,
        current: () => true,
        elapsed: () => now,
      );
      reply = (_) => pending.future;
      h.tap();
      await flush();
      now += const Duration(seconds: 1);
      for (var i = 0; i < 100; i++) {
        h.hold();
      }
      await flush();
      expect(calls.length, 1);
      pending.complete(null);
      reply = null;
      await flush();
      expect(calls.length, 1);
      h.hold();
      await flush();
      expect(calls.length, 2);
      now += const Duration(milliseconds: 74);
      h.tap();
      await flush();
      expect(calls.length, 2);
      now += const Duration(milliseconds: 1);
      h.tap();
      await flush();
      expect(calls.length, 3);
      h.dispose();
    },
  );

  test(
    'missing hardware and platform failures never escape into the input path',
    () async {
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      var now = Duration.zero;
      final h = MobileHaptics(
        enabled: () => true,
        current: () => true,
        elapsed: () => now,
      );
      reply = (_) async => throw MissingPluginException();
      h.tap();
      await flush();
      now += const Duration(milliseconds: 100);
      reply = (_) async => throw PlatformException(code: 'unavailable');
      h.hold();
      await flush();
      now += const Duration(milliseconds: 100);
      reply = null;
      h.tap();
      await flush();
      expect(calls.length, 3);
      h.dispose();
    },
  );

  testWidgets('a stalled haptics channel never delays remote click delivery', (
    tester,
  ) async {
    final engine = InputEngine();
    final active = MobileInput(engine, 'control', enabled: true);
    final h = MobileHaptics(enabled: () => true, current: () => true);
    final pending = Completer<Object?>();
    reply = (_) => pending.future;
    await tester.pumpWidget(
      MaterialApp(
        home: SizedBox.expand(
          child: MobileTouchSurface(
            input: active,
            mode: TouchMode.direct,
            videoSize: const Size(800, 600),
            onTapFeedback: h.tap,
            onHoldFeedback: h.hold,
          ),
        ),
      ),
    );
    await tester.tap(find.byKey(const ValueKey('mobile-touch-surface')));
    await tester.pump();
    await active.drain();
    expect(calls.length, 1);
    expect(engine.events.where((e) => e.$2 == 1).map((e) => e.$7), [
      true,
      false,
    ]);
    expect(engine.events.every((e) => e.$1 == 'control'), true);
    await tester.pumpWidget(const SizedBox.shrink());
    h.dispose();
    pending.complete(null);
    await tester.pump();
    active.dispose();
    await active.drain();
    expect(tester.takeException(), isNull);
  }, variant: TargetPlatformVariant.only(TargetPlatform.android));

  testWidgets(
    'tap, right-click and drag-start have feedback; moves and view-only do not',
    (tester) async {
      final engine = InputEngine(),
          input = MobileInput(engine, 's', enabled: true);
      var taps = 0, holds = 0;
      Future<void> show(MobileInput source, TouchMode mode) =>
          tester.pumpWidget(
            MaterialApp(
              home: SizedBox.expand(
                child: MobileTouchSurface(
                  input: source,
                  mode: mode,
                  videoSize: const Size(800, 600),
                  onTapFeedback: () => taps++,
                  onHoldFeedback: () => holds++,
                ),
              ),
            ),
          );
      await show(input, TouchMode.direct);
      final area = find.byKey(const ValueKey('mobile-touch-surface'));
      await tester.tapAt(const Offset(100, 100));
      await tester.longPress(area);
      expect(taps, 1);
      expect(holds, 1);
      final finger = await tester.startGesture(const Offset(100, 200));
      await finger.moveTo(const Offset(180, 200));
      await tester.pump();
      await finger.moveTo(const Offset(260, 200));
      await tester.pump();
      await finger.moveTo(const Offset(310, 200));
      await tester.pump();
      await finger.up();
      expect(holds, 2);
      await show(input, TouchMode.touchpad);
      await tester.drag(area, const Offset(100, 0));
      expect(taps, 1);
      expect(holds, 2);
      final viewer = MobileInput(engine, 'view', enabled: false);
      await show(viewer, TouchMode.pan);
      await tester.tap(area);
      await tester.longPress(area);
      await tester.drag(area, const Offset(100, 0));
      expect(taps, 1);
      expect(holds, 2);
      await tester.pumpWidget(const SizedBox.shrink());
      input.dispose();
      viewer.dispose();
      await input.drain();
      await viewer.drain();
    },
    variant: TargetPlatformVariant.only(TargetPlatform.android),
  );

  testWidgets('letterbox, cancelled touches and zoom never request feedback', (
    tester,
  ) async {
    final input = MobileInput(InputEngine(), 's', enabled: true);
    var count = 0;
    await tester.pumpWidget(
      MaterialApp(
        home: SizedBox.expand(
          child: MobileTouchSurface(
            input: input,
            mode: TouchMode.direct,
            videoSize: const Size(1600, 900),
            onTapFeedback: () => count++,
            onHoldFeedback: () => count++,
          ),
        ),
      ),
    );
    await tester.tapAt(const Offset(400, 1));
    final cancelled = await tester.startGesture(const Offset(100, 200));
    await cancelled.cancel();
    final a = await tester.startGesture(const Offset(300, 300), pointer: 1);
    final b = await tester.startGesture(const Offset(500, 300), pointer: 2);
    await a.moveTo(const Offset(250, 300));
    await b.moveTo(const Offset(550, 300));
    await tester.pump();
    await a.moveTo(const Offset(200, 300));
    await b.moveTo(const Offset(600, 300));
    await tester.pump();
    await a.up();
    await b.up();
    expect(count, 0);
    await tester.pumpWidget(const SizedBox.shrink());
    input.dispose();
    await input.drain();
  });

  testWidgets(
    'game feedback happens on press/deadzone exit, not move/release',
    (tester) async {
      final input = MobileInput(InputEngine(), 's', enabled: true);
      var count = 0;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: MobileGameControls(
              input: input,
              onPressFeedback: () => count++,
            ),
          ),
        ),
      );
      final finger = await tester.startGesture(
        tester.getCenter(find.byKey(const ValueKey('game-44'))),
      );
      expect(count, 1);
      await finger.cancel();
      expect(count, 1);
      final center = tester.getCenter(find.byKey(const ValueKey('game-stick')));
      final stick = await tester.startGesture(center);
      expect(count, 1);
      await stick.moveTo(center + const Offset(0, -25));
      await tester.pump();
      expect(count, 2);
      await stick.moveTo(center + const Offset(10, -30));
      await tester.pump();
      expect(count, 2);
      await stick.up();
      expect(count, 2);
      await tester.pumpWidget(const SizedBox.shrink());
      input.dispose();
      await input.drain();
    },
    variant: TargetPlatformVariant.only(TargetPlatform.android),
  );
}
