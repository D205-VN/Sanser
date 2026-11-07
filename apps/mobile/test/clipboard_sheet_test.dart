import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/clipboard_sheet.dart';
import 'package:sanser_mobile/src/native_engine.dart';
import 'package:sanser_mobile/src/theme.dart';

class ClipboardEngine extends NativeEngine {
  final requests = <(String, bool, String)>[];
  final reply = Completer<String>();
  @override
  Future<String> clipboard(String id, {required bool write, String text = ''}) {
    requests.add((id, write, text));
    return reply.future;
  }
}

void main() {
  test('clipboard validation uses bytes and preserves valid CRLF/Unicode', () {
    expect(validClipboardText('Tiếng Việt 😀\r\n\t'), isTrue);
    expect(validClipboardText('😀' * 2048), isTrue);
    expect(validClipboardText('ế' * 3000), isFalse);
    expect(validClipboardText('a\u0000b'), isFalse);
    expect(validClipboardText('a\ud800'), isFalse);
  });
  testWidgets(
    'manual read stays private until local Copy; repeated requests are blocked',
    (tester) async {
      final engine = ClipboardEngine();
      final calls = <MethodCall>[];
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        SystemChannels.platform,
        (call) async {
          calls.add(call);
          return null;
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          SystemChannels.platform,
          null,
        ),
      );
      await tester.pumpWidget(
        MaterialApp(
          theme: sanserTheme(),
          home: Scaffold(
            body: ClipboardSheet(
              engine: engine,
              sessionId: 'session-a',
              current: () => true,
            ),
          ),
        ),
      );
      expect(engine.requests, isEmpty);
      await tester.tap(find.text('Nhận từ máy tính'));
      await tester.pump();
      await tester.tap(find.text('Nhận từ máy tính'));
      expect(engine.requests, [('session-a', false, '')]);
      engine.reply.complete('private ế😀');
      await tester.pump();
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        'private ế😀',
      );
      expect(
        calls.where((call) => call.method == 'Clipboard.setData'),
        isEmpty,
      );
      await tester.tap(find.text('Sao chép vào điện thoại'));
      await tester.pump();
      expect(
        calls
            .where((call) => call.method == 'Clipboard.setData')
            .single
            .arguments,
        {'text': 'private ế😀'},
      );
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );
  testWidgets(
    'a response from a finished session cannot fill the next clipboard editor',
    (tester) async {
      var current = true;
      final engine = ClipboardEngine();
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: ClipboardSheet(
              engine: engine,
              sessionId: 'session-a',
              current: () => current,
            ),
          ),
        ),
      );
      await tester.tap(find.text('Nhận từ máy tính'));
      await tester.pump();
      current = false;
      engine.reply.complete('old secret');
      await tester.pump();
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        '',
      );
      expect(find.text('Phiên điều khiển đã kết thúc.'), findsOneWidget);
      await tester.pumpWidget(const SizedBox.shrink());
    },
  );
  testWidgets(
    'closing the sheet while a reply is pending cannot update disposed UI',
    (tester) async {
      final engine = ClipboardEngine();
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: ClipboardSheet(
              engine: engine,
              sessionId: 'session-a',
              current: () => true,
            ),
          ),
        ),
      );
      await tester.tap(find.text('Nhận từ máy tính'));
      await tester.pump();
      await tester.pumpWidget(const SizedBox.shrink());
      engine.reply.complete('closed');
      await tester.pump();
      expect(tester.takeException(), isNull);
    },
  );
}
