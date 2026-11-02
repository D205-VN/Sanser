import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/mobile_input.dart';
import 'package:sanser_mobile/src/native_engine.dart';
import 'package:sanser_mobile/src/touch_surface.dart';

class InputEngine extends NativeEngine {
  final events = <(String, int, int, int, int, int, bool)>[];
  final viewports = <(double, double, double)>[];
  @override
  Future<void> remoteInput(
    String sessionId,
    int kind, {
    int x = 0,
    int y = 0,
    int code = 0,
    int delta = 0,
    bool down = false,
  }) async {
    events.add((sessionId, kind, x, y, code, delta, down));
  }

  @override
  Future<void> viewport(
    String sessionId,
    double scale,
    double x,
    double y,
  ) async {
    viewports.add((scale, x, y));
  }
}

void main() {
  testWidgets(
    'direct drag presses at the original finger point before moving',
    (tester) async {
      final engine = InputEngine();
      final input = MobileInput(engine, 'drag', enabled: true);
      await tester.pumpWidget(
        MaterialApp(
          home: SizedBox.expand(
            child: MobileTouchSurface(
              input: input,
              mode: TouchMode.direct,
              videoSize: const Size(800, 600),
            ),
          ),
        ),
      );
      final finger = await tester.startGesture(const Offset(100, 200));
      await finger.moveTo(const Offset(180, 200));
      await tester.pump(const Duration(milliseconds: 30));
      await finger.moveTo(const Offset(250, 200));
      await tester.pump(const Duration(milliseconds: 30));
      await finger.up();
      await input.drain();
      final down = engine.events.firstWhere((e) => e.$2 == 1 && e.$7);
      expect(down.$3, (100 / 800 * 65535).round());
      expect(engine.events.last.$2, 1);
      expect(engine.events.last.$7, false);
      await tester.pumpWidget(const SizedBox.shrink());
      input.dispose();
      await input.drain();
    },
  );
  test('moves coalesce and flush before reliable button transitions', () async {
    final engine = InputEngine();
    final active = MobileInput(engine, 'session-a', enabled: true);
    active.move(const Offset(.1, .2));
    active.move(const Offset(.8, .7));
    await active.click(1);
    await active.drain();
    expect(engine.events.map((e) => e.$2), [0, 1, 1]);
    expect(engine.events.first.$3, (.8 * 65535).round());
    expect(engine.events.map((e) => e.$1).toSet(), {'session-a'});
    expect(engine.events[1].$5, 1);
    expect(engine.events[1].$7, true);
    expect(engine.events.last.$7, false);
    active.dispose();
    await active.drain();
  });
  test('independent held-key owners cannot release another control', () async {
    final engine = InputEngine();
    final active = MobileInput(engine, 'session-a', enabled: true);
    await active.hold(225, true, 'game');
    await active.hold(225, true, 'toolbar');
    await active.hold(225, false, 'game');
    expect(engine.events.length, 1);
    await active.hold(225, false, 'toolbar');
    expect(engine.events.map((e) => e.$7), [true, false]);
    active.dispose();
    await active.drain();
  });
  test(
    'view-only never emits input; teardown cancels pending moves and releases keys',
    () async {
      final engine = InputEngine(),
          viewer = MobileInput(engine, 'view', enabled: false);
      viewer.move(const Offset(.5, .5));
      await viewer.click();
      await viewer.hold(4, true, 'key');
      viewer.dispose();
      expect(engine.events, isEmpty);
      final active = MobileInput(engine, 'control', enabled: true);
      await active.hold(4, true, 'key');
      active.move(const Offset(.7, .7));
      active.dispose();
      await active.drain();
      expect(engine.events.map((e) => e.$2), [2, 4]);
    },
  );
  test('letterboxing and inverse viewport map to the real host point', () {
    const screen = Size(1000, 600), video = Size(1600, 900);
    expect(
      remotePosition(const Offset(500, 300), screen, video, 1, Offset.zero),
      const Offset(.5, .5),
    );
    expect(
      remotePosition(const Offset(10, 1), screen, video, 1, Offset.zero),
      isNull,
    );
    expect(
      remotePosition(
        const Offset(700, 350),
        screen,
        video,
        2,
        const Offset(200, 50),
      ),
      const Offset(.5, .5),
    );
  });
  testWidgets(
    'touchpad moves, long-press right-clicks, and background releases held input',
    (tester) async {
      final engine = InputEngine();
      final active = MobileInput(engine, 'session-a', enabled: true);
      await tester.pumpWidget(
        MaterialApp(
          home: SizedBox.expand(
            child: MobileTouchSurface(
              input: active,
              mode: TouchMode.touchpad,
              videoSize: const Size(1920, 1080),
            ),
          ),
        ),
      );
      await tester.drag(
        find.byKey(const ValueKey('mobile-touch-surface')),
        const Offset(80, 10),
      );
      await tester.pump(const Duration(milliseconds: 20));
      await active.drain();
      expect(engine.events.where((e) => e.$2 == 0), isNotEmpty);
      await tester.longPress(
        find.byKey(const ValueKey('mobile-touch-surface')),
      );
      await active.drain();
      expect(
        engine.events.where((e) => e.$2 == 1 && e.$5 == 1 && e.$7),
        hasLength(1),
      );
      await active.hold(26, true, 'stick');
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      await active.drain();
      expect(engine.events.last.$2, 4);
      await tester.pumpWidget(const SizedBox.shrink());
      active.dispose();
      await active.drain();
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
    },
  );
  testWidgets(
    'pinch zoom changes only the local viewport, never clicks the host',
    (tester) async {
      final engine = InputEngine();
      final active = MobileInput(engine, 'session-a', enabled: true);
      await tester.pumpWidget(
        MaterialApp(
          home: SizedBox.expand(
            child: MobileTouchSurface(
              input: active,
              mode: TouchMode.pan,
              videoSize: const Size(1920, 1080),
            ),
          ),
        ),
      );
      final a = await tester.startGesture(const Offset(300, 300), pointer: 1),
          b = await tester.startGesture(const Offset(500, 300), pointer: 2);
      await a.moveTo(const Offset(250, 300));
      await b.moveTo(const Offset(550, 300));
      await tester.pump(const Duration(milliseconds: 30));
      await a.moveTo(const Offset(200, 300));
      await b.moveTo(const Offset(600, 300));
      await tester.pump(const Duration(milliseconds: 30));
      await a.up();
      await b.up();
      await active.drain();
      expect(engine.viewports.any((v) => v.$1 > 1), true);
      expect(engine.events, isEmpty);
      await tester.pumpWidget(const SizedBox.shrink());
      active.dispose();
      await active.drain();
    },
  );
  testWidgets('game key sends release when its pointer is cancelled', (
    tester,
  ) async {
    final engine = InputEngine();
    final active = MobileInput(engine, 'session-a', enabled: true);
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(body: MobileGameControls(input: active)),
      ),
    );
    final finger = await tester.startGesture(
      tester.getCenter(find.byKey(const ValueKey('game-44'))),
    );
    await active.drain();
    expect(engine.events.last.$5, 44);
    expect(engine.events.last.$7, true);
    await finger.cancel();
    await active.drain();
    expect(engine.events.last.$7, false);
    await tester.pumpWidget(const SizedBox.shrink());
    active.dispose();
    await active.drain();
  });
}
