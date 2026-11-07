import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/api.dart';
import 'package:sanser_mobile/src/controller.dart';
import 'package:sanser_mobile/src/floating_controls.dart';
import 'package:sanser_mobile/src/native_engine.dart';
import 'package:sanser_mobile/src/screens.dart';
import 'package:sanser_mobile/src/theme.dart';
import 'widget_test.dart' show MemorySecrets;

class ActionEngine extends NativeEngine {
  final input = <(String, int, int, bool)>[];
  final mutes = <bool>[];
  Completer<void>? muteReply;
  final edits = <(String, String)>[];
  Completer<bool>? textReply;
  @override
  Future<bool> text(String id, String text) async {
    edits.add((id, text));
    return await textReply?.future ?? true;
  }

  @override
  Stream<Json> get updates => const Stream.empty();
  @override
  Future<void> mute(bool muted) async {
    mutes.add(muted);
    await muteReply?.future;
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
  }) async => input.add((id, kind, code, down));
  @override
  Future<void> viewport(String id, double scale, double x, double y) async {}
  @override
  Future<void> resetInput() async {}
  @override
  Future<void> disconnect() async {}
}

MobileController active(ActionEngine engine, {bool viewOnly = false}) =>
    MobileController(SanserApi(MemorySecrets()), engine)
      ..account = {'email': 'test@example.com'}
      ..active = {
        'id': 'session-a',
        'accessMode': viewOnly ? 'view-only' : 'control',
      }
      ..phase = SessionPhase.streaming
      ..selectedTransport = 'direct'
      ..capabilities = {'available': true, 'audio': true, 'voice': false};

Future<void> showControls(
  WidgetTester tester,
  MobileController c, {
  double textScale = 1,
}) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = const Size(390, 844);
  addTearDown(tester.view.resetDevicePixelRatio);
  addTearDown(tester.view.resetPhysicalSize);
  await tester.pumpWidget(
    MaterialApp(
      theme: sanserTheme(),
      builder: (context, child) => MediaQuery(
        data: MediaQuery.of(
          context,
        ).copyWith(textScaler: TextScaler.linear(textScale)),
        child: child!,
      ),
      home: SanserHome(controller: c),
    ),
  );
  await tester.pump(const Duration(milliseconds: 100));
  await tester.tap(find.byTooltip('Mở điều khiển · kéo để di chuyển'));
  await tester.pump(const Duration(milliseconds: 100));
}

Future<void> finish(WidgetTester tester, MobileController c) async {
  c.active = null;
  await tester.pumpWidget(const SizedBox.shrink());
  c.dispose();
  await tester.pump(const Duration(milliseconds: 500));
}

void main() {
  testWidgets(
    'view-only actions allow local viewing but expose no keyboard or game controls',
    (tester) async {
      final engine = ActionEngine();
      final viewer = active(engine, viewOnly: true);
      await showControls(tester, viewer, textScale: 1.5);
      expect(find.text('Bàn phím'), findsNothing);
      expect(find.text('Bàn phím ngoài'), findsNothing);
      expect(find.text('Phím và điều khiển'), findsNothing);
      expect(find.byTooltip('Touchpad'), findsNothing);
      expect(find.text('Cài đặt phiên'), findsOneWidget);
      expect(engine.input, isEmpty);
      expect(tester.takeException(), isNull);
      await finish(tester, viewer);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets(
    'pending and audio-incapable sessions cannot send input or mute commands',
    (tester) async {
      final engine = ActionEngine();
      final pending = active(engine)
        ..phase = SessionPhase.approval
        ..capabilities = {
          'available': true,
          'audio': false,
          'controllerKeys': true,
        };
      await showControls(tester, pending);
      expect(
        tester
            .widget<OutlinedButton>(
              find.widgetWithText(OutlinedButton, 'Bàn phím'),
            )
            .onPressed,
        isNull,
      );
      expect(
        tester
            .widget<OutlinedButton>(
              find.widgetWithText(OutlinedButton, 'Bàn phím ngoài'),
            )
            .onPressed,
        isNull,
      );
      expect(find.text('Tắt âm thanh'), findsNothing);
      expect(
        tester
            .widget<OutlinedButton>(
              find.widgetWithText(OutlinedButton, 'Tay cầm → phím'),
            )
            .onPressed,
        isNull,
      );
      await pending.setSound(false);
      expect(engine.mutes, isEmpty);
      expect(engine.input, isEmpty);
      expect(tester.takeException(), isNull);
      await finish(tester, pending);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets(
    'external keys need opt-in and emergency release disables capture',
    (tester) async {
      final engine = ActionEngine();
      final c = active(engine);
      await showControls(tester, c);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA);
      await tester.pump();
      expect(engine.input.where((e) => e.$2 == 2), isEmpty);
      final action = find.widgetWithText(OutlinedButton, 'Bàn phím ngoài');
      await tester.ensureVisible(action);
      await tester.tap(action);
      await tester.pump();
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyA);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyA);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();
      expect(engine.input.where((e) => e.$2 == 2).toList(), [
        ('session-a', 2, 225, true),
        ('session-a', 2, 4, true),
        ('session-a', 2, 4, false),
        ('session-a', 2, 225, false),
      ]);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyDownEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      await tester.sendKeyUpEvent(LogicalKeyboardKey.escape);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump();
      expect(engine.input.where((e) => e.$2 == 2 && e.$3 == 41), isEmpty);
      expect(
        find.text(
          'Bấm vào hình để gửi phím USB/Bluetooth. Ctrl+Alt+Esc để nhả.',
        ),
        findsNothing,
      );
      final count = engine.input.length;
      await tester.sendKeyDownEvent(LogicalKeyboardKey.keyB);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.keyB);
      await tester.pump();
      expect(engine.input.length, count);
      expect(tester.takeException(), isNull);
      await finish(tester, c);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets(
    'a pending sound action blocks repeat taps and updates only after acknowledgement',
    (tester) async {
      final engine = ActionEngine()..muteReply = Completer<void>();
      final c = active(engine);
      await showControls(tester, c);
      final mute = find.widgetWithText(OutlinedButton, 'Tắt âm thanh');
      await tester.tap(mute);
      await tester.pump();
      expect(tester.widget<OutlinedButton>(mute).onPressed, isNull);
      expect(c.sound, true);
      expect(engine.mutes, [true]);
      engine.muteReply!.complete();
      await tester.pump();
      await tester.pump();
      expect(c.sound, false);
      expect(find.text('Bật âm thanh'), findsOneWidget);
      await finish(tester, c);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets('changing session identity creates new keyboard ownership', (
    tester,
  ) async {
    final engine = ActionEngine();
    final c = active(engine);
    await showControls(tester, c);
    c.active = {'id': 'session-b', 'accessMode': 'control'};
    c.notifyListeners();
    await tester.pump(const Duration(milliseconds: 100));
    await tester.tap(find.byTooltip('Mở điều khiển · kéo để di chuyển'));
    await tester.pump();
    await tester.tap(find.widgetWithText(OutlinedButton, 'Bàn phím'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(OutlinedButton, 'Esc'));
    await tester.pumpAndSettle();
    final keys = engine.input
        .where((event) => event.$2 == 2 && event.$3 == 41)
        .toList();
    expect(keys.map((event) => event.$1), ['session-b', 'session-b']);
    expect(keys.map((event) => event.$4), [true, false]);
    Navigator.of(tester.element(find.text('Bàn phím từ xa'))).pop();
    await tester.pumpAndSettle();
    await tester.pump(const Duration(milliseconds: 500));
    await finish(tester, c);
  }, variant: TargetPlatformVariant.only(TargetPlatform.linux));

  testWidgets(
    'collapse and disconnect remain clickable after scrolling a long action list',
    (tester) async {
      var stops = 0;
      await tester.pumpWidget(
        MaterialApp(
          theme: sanserTheme(),
          home: Scaffold(
            body: FloatingSessionControls(
              initiallyCompact: false,
              footer: OutlinedButton(
                onPressed: () => stops++,
                child: const Text('Disconnect preview'),
              ),
              content: Column(
                children: [
                  for (var i = 0; i < 30; i++)
                    SizedBox(height: 48, child: Text('Action $i')),
                ],
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.drag(
        find.byType(SingleChildScrollView),
        const Offset(0, -400),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Disconnect preview'));
      expect(stops, 1);
      await tester.tap(find.byTooltip('Thu gọn điều khiển'));
      await tester.pumpAndSettle();
      expect(
        find.byTooltip('Mở điều khiển · kéo để di chuyển'),
        findsOneWidget,
      );
      expect(tester.takeException(), isNull);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  testWidgets(
    'Unicode editor sends once while pending and keeps the original session',
    (tester) async {
      final engine = ActionEngine()..textReply = Completer<bool>();
      final c = active(engine)..telemetry = {'unicodeInput': true};
      await showControls(tester, c);
      await tester.tap(find.text('Bàn phím'));
      await tester.pumpAndSettle();
      expect(find.text('Nhập văn bản · tiếng Việt và emoji'), findsOneWidget);
      await tester.enterText(find.byType(TextField), 'Tiếng Việt 😀');
      await tester.pump();
      final send = find.widgetWithText(FilledButton, 'Gửi');
      await tester.ensureVisible(send);
      await tester.tap(send);
      await tester.pump();
      expect(find.text('Đang gửi…'), findsOneWidget);
      await tester.tap(find.text('Đang gửi…'));
      expect(engine.edits, [('session-a', 'Tiếng Việt 😀')]);
      engine.textReply!.complete(true);
      await tester.pumpAndSettle();
      expect(find.text('Bàn phím từ xa'), findsNothing);
      await finish(tester, c);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.linux),
  );

  test(
    'late audio acknowledgement and error cannot change the next session',
    () async {
      for (final fail in [false, true]) {
        final engine = ActionEngine()..muteReply = Completer<void>();
        final c = active(engine);
        final old = c.setSound(false);
        c.active = {'id': 'session-b', 'accessMode': 'control'};
        c.sound = true;
        if (fail) {
          engine.muteReply!.completeError(
            StateError('Old session audio failed'),
          );
        } else {
          engine.muteReply!.complete();
        }
        await old;
        expect(c.sound, true);
        expect(c.error, isNull);
        c.active = null;
        c.dispose();
      }
    },
  );
}
