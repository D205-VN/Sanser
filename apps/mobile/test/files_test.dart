import 'dart:async';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/api.dart';
import 'package:sanser_mobile/src/files.dart';
import 'package:sanser_mobile/src/native_engine.dart';
import 'package:sanser_mobile/src/theme.dart';

class FileEngine extends NativeEngine {
  final actions = <Json>[];
  final exports = <String>[];
  Json reply = {'available': true, 'incoming': null, 'outgoing': null};
  Completer<Json>? pending;
  Completer<Json?>? picker;
  @override
  Future<Json> files(String id, Json action) async {
    actions.add({'sessionId': id, ...action});
    return await pending?.future ?? reply;
  }

  @override
  Future<Json?> pickFile(String id) async =>
      await picker?.future ??
      {'sourceId': 'source-a', 'name': 'notes.txt', 'size': 5};
  @override
  Future<List<Json>> fileInbox() async => [
    {'key': 'owned/file', 'name': 'notes.txt', 'size': 5},
  ];
  @override
  Future<bool> exportFile(String key) async {
    exports.add(key);
    return true;
  }
}

Future<void> show(WidgetTester tester, Widget child) async {
  await tester.pumpWidget(
    MaterialApp(
      theme: sanserTheme(),
      home: Scaffold(body: child),
    ),
  );
  await tester.pump();
}

const incoming = {
  'id': 'file-a',
  'name': 'notes.txt',
  'size': 5,
  'bytes': 0,
  'state': 'offered',
  'error': null,
  'savedPath': null,
};
void main() {
  testWidgets('invitation waits for explicit consent and can be declined', (
    tester,
  ) async {
    final engine = FileEngine()
      ..reply = {'available': true, 'incoming': incoming, 'outgoing': null};
    final files = MobileFiles(
      engine,
      'session-a',
      current: () => true,
      start: false,
    );
    await files.refresh();
    engine.actions.clear();
    await show(tester, FileTransfersSheet(files: files));
    expect(engine.actions, isEmpty);
    await tester.tap(find.text('Chấp nhận'));
    await tester.pump();
    expect(engine.actions.single, {
      'sessionId': 'session-a',
      'operation': 'accept',
      'id': 'file-a',
    });
    await tester.tap(find.text('Từ chối'));
    await tester.pump();
    expect(engine.actions.last['operation'], 'cancel');
    await tester.pumpWidget(const SizedBox());
    files.dispose();
  });
  testWidgets(
    'picker sends only a source token and metadata after explicit offer',
    (tester) async {
      final engine = FileEngine();
      final files = MobileFiles(
        engine,
        'session-a',
        current: () => true,
        start: false,
      );
      await files.refresh();
      engine.actions.clear();
      await show(tester, FileTransfersSheet(files: files));
      await tester.tap(find.text('Chọn file'));
      await tester.pump();
      expect(engine.actions, isEmpty);
      await tester.tap(find.text('Mời nhận file'));
      await tester.pump();
      expect(engine.actions.single, {
        'sessionId': 'session-a',
        'operation': 'offer',
        'sourceId': 'source-a',
        'name': 'notes.txt',
      });
      await tester.pumpWidget(const SizedBox());
      files.dispose();
    },
  );
  test('pending and old-session replies cannot update a new session', () async {
    var current = true;
    final engine = FileEngine();
    final files = MobileFiles(
      engine,
      'session-a',
      current: () => current,
      start: false,
    );
    await files.refresh();
    engine.actions.clear();
    engine.pending = Completer<Json>();
    final first = files.act({'operation': 'accept', 'id': 'file-a'});
    await files.act({'operation': 'accept', 'id': 'file-a'});
    expect(engine.actions.length, 1);
    current = false;
    engine.pending!.complete({
      'available': true,
      'incoming': {'name': 'old-secret'},
      'outgoing': null,
    });
    await first;
    expect(files.incoming, isNull);
    files.dispose();
  });
  testWidgets('verified inbox exports only after tapping Save', (tester) async {
    final engine = FileEngine();
    await show(tester, ReceivedFiles(engine: engine));
    await tester.pump();
    expect(find.text('notes.txt'), findsOneWidget);
    expect(engine.exports, isEmpty);
    await tester.tap(find.text('Lưu'));
    await tester.pump();
    expect(engine.exports, ['owned/file']);
    await tester.pumpWidget(const SizedBox());
  });
  testWidgets('unsupported capabilities cannot offer or accept files', (
    tester,
  ) async {
    final engine = FileEngine()
      ..reply = {'available': false, 'incoming': incoming, 'outgoing': null};
    final files = MobileFiles(
      engine,
      'session-a',
      current: () => true,
      start: false,
    );
    await files.refresh();
    engine.actions.clear();
    await show(tester, FileTransfersSheet(files: files));
    await tester.tap(find.text('Chọn file'));
    await tester.tap(find.text('Chấp nhận'));
    await tester.pump();
    expect(engine.actions, isEmpty);
    await tester.pumpWidget(const SizedBox());
    files.dispose();
  });
  testWidgets('document UI lease ends on cancellation and error', (
    tester,
  ) async {
    final engine = NativeEngine();
    final result = Completer<Object?>();
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      NativeEngine.channel,
      (_) => result.future,
    );
    final picking = engine.pickFile('session-a');
    expect(engine.documentUIActive, isTrue);
    result.complete(null);
    expect(await picking, isNull);
    expect(engine.documentUIActive, isFalse);
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      NativeEngine.channel,
      (_) => throw PlatformException(code: 'cancelled'),
    );
    await expectLater(
      engine.exportFile('owned/file'),
      throwsA(isA<PlatformException>()),
    );
    expect(engine.documentUIActive, isFalse);
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      NativeEngine.channel,
      null,
    );
  });
}
