import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/keyboard_sheet.dart';
import 'package:sanser_mobile/src/mobile_input.dart';
import 'package:sanser_mobile/src/native_engine.dart';

class ImeEngine extends NativeEngine {
  final edits = <(String, String)>[];
  final keys = <(int, int, bool)>[];
  Completer<bool>? reply;
  Completer<void>? reset;
  @override
  Future<bool> text(String sessionId, String text) async {
    edits.add((sessionId, text));
    return await reply?.future ?? true;
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
    keys.add((kind, code, down));
    if (kind == 4) await reset?.future;
  }
}

Future<MobileInput> open(
  WidgetTester tester,
  ImeEngine engine, {
  bool unicode = true,
  bool Function()? current,
}) async {
  tester.view.devicePixelRatio = 1;
  tester.view.physicalSize = const Size(420, 900);
  addTearDown(tester.view.resetDevicePixelRatio);
  addTearDown(tester.view.resetPhysicalSize);
  final input = MobileInput(engine, 'session-a', enabled: true);
  await tester.pumpWidget(
    MaterialApp(
      home: Scaffold(
        body: Builder(
          builder: (context) => TextButton(
            onPressed: () => showModalBottomSheet<void>(
              context: context,
              isScrollControlled: true,
              builder: (_) => RemoteKeyboardSheet(
                input: input,
                current: current ?? () => true,
                unicodeSupported: unicode,
              ),
            ),
            child: const Text('Open'),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Open'));
  await tester.pumpAndSettle();
  return input;
}

Future<void> edit(
  WidgetTester tester,
  String value, {
  TextRange? composing,
  TextSelection? selection,
}) async {
  tester.testTextInput.updateEditingValue(
    TextEditingValue(
      text: value,
      selection: selection ?? TextSelection.collapsed(offset: value.length),
      composing: composing ?? TextRange.empty,
    ),
  );
  await tester.pump();
}

void main() {
  testWidgets(
    'IME candidates and selection stay local until committed; remote keys cannot interrupt',
    (tester) async {
      final engine = ImeEngine();
      await open(tester, engine);
      await edit(tester, 'tiee', composing: const TextRange(start: 0, end: 4));
      await edit(tester, 'tiế', composing: const TextRange(start: 0, end: 3));
      expect(
        tester
            .widget<FilledButton>(find.widgetWithText(FilledButton, 'Gửi'))
            .onPressed,
        isNull,
      );
      expect(
        tester
            .widget<OutlinedButton>(
              find.widgetWithText(OutlinedButton, 'Enter'),
            )
            .onPressed,
        isNull,
      );
      expect(
        tester
            .widget<OutlinedButton>(
              find.widgetWithText(OutlinedButton, 'Ctrl+V'),
            )
            .onPressed,
        isNull,
      );
      expect(engine.edits, isEmpty);
      expect(engine.keys, isEmpty);
      const selected = TextSelection(baseOffset: 0, extentOffset: 5);
      await edit(tester, 'tiếng', selection: selected);
      final controller = tester
          .widget<TextField>(find.byType(TextField))
          .controller!;
      expect(controller.selection, selected);
      await tester.tap(find.text('Gửi'));
      await tester.pumpAndSettle();
      expect(engine.edits, [('session-a', 'tiếng')]);
      expect(engine.keys, [(4, 0, false)]);
      expect(find.text('Bàn phím từ xa'), findsNothing);
    },
  );

  testWidgets(
    'explicit Chốt chữ ends composition but never sends or changes the draft',
    (tester) async {
      final engine = ImeEngine();
      await open(tester, engine);
      const value = 'e\u0301 👨‍👩‍👧‍👦';
      await edit(
        tester,
        value,
        composing: const TextRange(start: 0, end: 2),
        selection: const TextSelection.collapsed(offset: 2),
      );
      await tester.tap(find.text('Chốt chữ'));
      await tester.pumpAndSettle();
      final controller = tester
          .widget<TextField>(find.byType(TextField))
          .controller!;
      expect(controller.text, value);
      expect(controller.value.composing.isCollapsed, isTrue);
      expect(engine.edits, isEmpty);
      await tester.tap(find.text('Gửi'));
      await tester.pumpAndSettle();
      expect(engine.edits.single.$2, value);
    },
  );

  testWidgets(
    'UTF16 limit rejects whole emoji draft without truncating an IME candidate',
    (tester) async {
      final engine = ImeEngine();
      await open(tester, engine);
      final value = '😀' * 129;
      await edit(tester, value, composing: const TextRange(start: 0, end: 258));
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        value,
      );
      await edit(tester, value);
      expect(
        find.text('Văn bản quá dài. Chia thành đoạn ngắn hơn.'),
        findsOneWidget,
      );
      expect(
        tester.widget<FilledButton>(find.byType(FilledButton)).onPressed,
        isNull,
      );
      expect(engine.edits, isEmpty);
    },
  );

  testWidgets(
    'native rejection retains exact selection and never retries a queued text edit',
    (tester) async {
      final engine = ImeEngine()..reply = Completer<bool>();
      await open(tester, engine);
      const selection = TextSelection(baseOffset: 0, extentOffset: 2);
      await edit(tester, 'ế 😀', selection: selection);
      await tester.tap(find.text('Gửi'));
      await tester.pump();
      await tester.tap(find.text('Đang gửi…'));
      expect(engine.edits.length, 1);
      engine.reply!.complete(false);
      await tester.pumpAndSettle();
      final value = tester
          .widget<TextField>(find.byType(TextField))
          .controller!
          .value;
      expect(value.text, 'ế 😀');
      expect(value.selection, selection);
      expect(engine.edits.length, 1);
      expect(find.textContaining('Chưa gửi được chữ:'), findsOneWidget);
    },
  );

  testWidgets('disconnect during input drain cancels the text edit', (
    tester,
  ) async {
    final engine = ImeEngine()..reset = Completer<void>();
    var current = true;
    await open(tester, engine, current: () => current);
    await edit(tester, 'không gửi sang phiên sau');
    await tester.tap(find.text('Gửi'));
    await tester.pump();
    current = false;
    engine.reset!.complete();
    await tester.pumpAndSettle();
    expect(engine.edits, isEmpty);
    expect(
      find.text('Phiên điều khiển đã kết thúc. Bản nháp chưa gửi.'),
      findsOneWidget,
    );
  });

  testWidgets('closed editor ignores a late text reply', (tester) async {
    final engine = ImeEngine()..reply = Completer<bool>();
    await open(tester, engine);
    await edit(tester, 'Tiếng Việt');
    await tester.tap(find.text('Gửi'));
    await tester.pump();
    await tester.pumpWidget(const SizedBox.shrink());
    engine.reply!.complete(true);
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(engine.edits.length, 1);
  });

  testWidgets(
    'old host never receives a Unicode prefix and only accepts supported ASCII',
    (tester) async {
      final engine = ImeEngine();
      await open(tester, engine, unicode: false);
      await edit(tester, 'aế');
      expect(
        tester.widget<FilledButton>(find.byType(FilledButton)).onPressed,
        isNull,
      );
      expect(engine.edits, isEmpty);
      expect(engine.keys, isEmpty);
    },
  );
}
