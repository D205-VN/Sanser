import 'dart:async';
import 'dart:ui' as ui;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/external_keyboard.dart';
import 'package:sanser_mobile/src/mobile_input.dart';
import 'package:sanser_mobile/src/native_engine.dart';

class KeyEngine extends NativeEngine {
  final events = <(String, int, int, bool)>[];
  Completer<void>? block;
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
    events.add((id, kind, code, down));
    await block?.future;
  }
}

KeyEvent down(
  PhysicalKeyboardKey key, {
  bool synthesized = false,
  ui.KeyEventDeviceType device = ui.KeyEventDeviceType.keyboard,
}) => KeyDownEvent(
  physicalKey: key,
  logicalKey: LogicalKeyboardKey.keyA,
  timeStamp: Duration.zero,
  synthesized: synthesized,
  deviceType: device,
);
KeyEvent up(PhysicalKeyboardKey key, {bool synthesized = false}) => KeyUpEvent(
  physicalKey: key,
  logicalKey: LogicalKeyboardKey.keyA,
  timeStamp: Duration.zero,
  synthesized: synthesized,
);
KeyEvent repeat(PhysicalKeyboardKey key) => KeyRepeatEvent(
  physicalKey: key,
  logicalKey: LogicalKeyboardKey.keyA,
  timeStamp: Duration.zero,
);

void main() {
  test(
    'HID allowlist matches native keys and excludes consumer/system pages',
    () {
      for (final key in [
        PhysicalKeyboardKey.keyA,
        PhysicalKeyboardKey.shiftRight,
        PhysicalKeyboardKey.controlLeft,
        PhysicalKeyboardKey.metaLeft,
        PhysicalKeyboardKey.arrowLeft,
        PhysicalKeyboardKey.numpad1,
        PhysicalKeyboardKey.f12,
      ]) {
        expect(externalKeyboardHid(key), key.usbHidUsage & 0xffff);
      }
      for (final key in [
        PhysicalKeyboardKey.audioVolumeUp,
        const PhysicalKeyboardKey(0x00070032),
        const PhysicalKeyboardKey(0x00010004),
      ]) {
        expect(externalKeyboardHid(key), isNull);
      }
    },
  );

  test(
    'physical down/repeat/up stays ordered and never sends character text',
    () async {
      final engine = KeyEngine(),
          input = MobileInput(engine, 'old', enabled: true);
      final keys = ExternalKeyboardInput(input, current: () => true);
      keys.handle(down(PhysicalKeyboardKey.keyA));
      await input.drain();
      keys.handle(repeat(PhysicalKeyboardKey.keyA));
      await input.drain();
      keys.handle(up(PhysicalKeyboardKey.keyA));
      await input.drain();
      expect(engine.events, [
        ('old', 2, 4, true),
        ('old', 2, 4, true),
        ('old', 2, 4, false),
      ]);
      keys.dispose();
      input.dispose();
      await input.drain();
    },
  );

  test(
    'no synthesized down, unowned repeat, game controller or modifier repeat',
    () async {
      final engine = KeyEngine(),
          input = MobileInput(engine, 's', enabled: true);
      final keys = ExternalKeyboardInput(input, current: () => true);
      expect(
        keys.handle(down(PhysicalKeyboardKey.keyA, synthesized: true)),
        KeyEventResult.ignored,
      );
      expect(
        keys.handle(repeat(PhysicalKeyboardKey.keyA)),
        KeyEventResult.ignored,
      );
      expect(
        keys.handle(
          down(PhysicalKeyboardKey.keyA, device: ui.KeyEventDeviceType.gamepad),
        ),
        KeyEventResult.ignored,
      );
      keys.handle(down(PhysicalKeyboardKey.shiftLeft));
      keys.handle(repeat(PhysicalKeyboardKey.shiftLeft));
      keys.handle(up(PhysicalKeyboardKey.shiftLeft, synthesized: true));
      await input.drain();
      expect(engine.events, [('s', 2, 225, true), ('s', 2, 225, false)]);
      keys.dispose();
      input.dispose();
      await input.drain();
    },
  );

  test('losing keyboard focus releases only its own Shift owner', () async {
    final engine = KeyEngine(), input = MobileInput(engine, 's', enabled: true);
    final keys = ExternalKeyboardInput(input, current: () => true);
    await input.hold(225, true, 'game');
    keys.handle(down(PhysicalKeyboardKey.shiftLeft));
    keys.release();
    await input.drain();
    expect(engine.events, [('s', 2, 225, true)]);
    await input.hold(225, false, 'game');
    expect(engine.events.last, ('s', 2, 225, false));
    keys.dispose();
    input.dispose();
    await input.drain();
  });

  test('reset prevents later repeats resurrecting a key', () async {
    final engine = KeyEngine(), input = MobileInput(engine, 's', enabled: true);
    final keys = ExternalKeyboardInput(input, current: () => true);
    keys.handle(down(PhysicalKeyboardKey.keyA));
    await input.releaseAll();
    keys.handle(repeat(PhysicalKeyboardKey.keyA));
    keys.handle(up(PhysicalKeyboardKey.keyA));
    await input.drain();
    expect(engine.events, [('s', 2, 4, true), ('s', 4, 0, false)]);
    keys.dispose();
    input.dispose();
    await input.drain();
  });

  test('slow bridge allows only one pending repeat per press', () async {
    final engine = KeyEngine(), input = MobileInput(engine, 's', enabled: true);
    final keys = ExternalKeyboardInput(input, current: () => true);
    keys.handle(down(PhysicalKeyboardKey.keyA));
    await input.drain();
    engine.block = Completer<void>();
    for (var i = 0; i < 100; i++) {
      keys.handle(repeat(PhysicalKeyboardKey.keyA));
    }
    await Future<void>.delayed(Duration.zero);
    expect(engine.events.length, 2);
    keys.release();
    engine.block!.complete();
    engine.block = null;
    await input.drain();
    expect(engine.events, [
      ('s', 2, 4, true),
      ('s', 2, 4, true),
      ('s', 2, 4, false),
    ]);
    keys.dispose();
    input.dispose();
    await input.drain();
  });

  test('Ctrl Alt Escape releases capture without forwarding Escape', () async {
    final engine = KeyEngine(), input = MobileInput(engine, 's', enabled: true);
    var released = 0;
    final keys = ExternalKeyboardInput(
      input,
      current: () => true,
      onReleased: () => released++,
    );
    keys.handle(down(PhysicalKeyboardKey.controlLeft));
    keys.handle(down(PhysicalKeyboardKey.altRight));
    keys.handle(down(PhysicalKeyboardKey.escape));
    await input.drain();
    expect(released, 1);
    expect(engine.events.where((e) => e.$3 == 41), isEmpty);
    expect(engine.events.map((e) => (e.$2, e.$3, e.$4)), [
      (2, 224, true),
      (2, 230, true),
      (2, 224, false),
      (2, 230, false),
      (4, 0, false),
    ]);
    keys.dispose();
    input.dispose();
    await input.drain();
  });

  test('readiness loss and read-only input cannot inject', () async {
    final engine = KeyEngine(), input = MobileInput(engine, 's', enabled: true);
    var ready = true;
    final keys = ExternalKeyboardInput(input, current: () => ready);
    keys.handle(down(PhysicalKeyboardKey.keyA));
    ready = false;
    expect(keys.handle(down(PhysicalKeyboardKey.keyB)), KeyEventResult.ignored);
    await input.drain();
    expect(engine.events, [('s', 2, 4, true), ('s', 2, 4, false)]);
    keys.dispose();
    input.dispose();
    await input.drain();
    engine.events.clear();
    final viewer = MobileInput(engine, 'view', enabled: false);
    final disabled = ExternalKeyboardInput(viewer, current: () => true);
    disabled.handle(down(PhysicalKeyboardKey.keyA));
    disabled.dispose();
    viewer.dispose();
    await viewer.drain();
    expect(engine.events, isEmpty);
  });

  testWidgets(
    'opt-in scope releases on sheet, background and session replacement',
    (tester) async {
      final engine = KeyEngine(),
          input = MobileInput(engine, 'first', enabled: true);
      var enabled = false;
      var current = true;
      late StateSetter update;
      late BuildContext routeContext;
      var selectedInput = input;
      await tester.pumpWidget(
        MaterialApp(
          home: StatefulBuilder(
            builder: (context, setState) {
              update = setState;
              routeContext = context;
              return Scaffold(
                body: ExternalKeyboardSurface(
                  input: selectedInput,
                  enabled: enabled,
                  current: () => current,
                  onReleased: () => update(() => enabled = false),
                  child: const SizedBox.expand(
                    child: ColoredBox(
                      color: Colors.black,
                      key: ValueKey('stream'),
                    ),
                  ),
                ),
              );
            },
          ),
        ),
      );
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA);
      await input.drain();
      expect(engine.events, isEmpty);
      update(() => enabled = true);
      await tester.pump();
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await input.drain();
      expect(engine.events.last, ('first', 2, 225, true));
      unawaited(
        showModalBottomSheet<void>(
          context: routeContext,
          builder: (_) => const TextField(autofocus: true),
        ),
      );
      await tester.pumpAndSettle();
      await input.drain();
      expect(engine.events.last, ('first', 2, 225, false));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      final before = engine.events.length;
      await tester.enterText(find.byType(TextField), 'local secret');
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyB);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyB);
      await input.drain();
      expect(engine.events.length, before);
      Navigator.of(tester.element(find.byType(TextField))).pop();
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('stream')));
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyC);
      await input.drain();
      expect(engine.events.last, ('first', 2, 6, true));
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.inactive);
      await tester.pump();
      await input.drain();
      expect(engine.events.last, ('first', 2, 6, false));
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pump();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyC);
      final count = engine.events.length;
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyD);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyD);
      await input.drain();
      expect(engine.events.length, count);
      await tester.tap(find.byKey(const ValueKey('stream')));
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyE);
      await input.drain();
      final next = MobileInput(engine, 'next', enabled: true);
      update(() => selectedInput = next);
      await tester.pump();
      await tester.pump();
      await input.drain();
      expect(engine.events.last, ('first', 2, 8, false));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyE);
      current = false;
      update(() {});
      await tester.pump();
      final last = engine.events.length;
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyF);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyF);
      await next.drain();
      expect(engine.events.length, last);
      await tester.pumpWidget(const SizedBox.shrink());
      input.dispose();
      next.dispose();
      await input.drain();
      await next.drain();
      expect(tester.takeException(), isNull);
    },
  );
}
