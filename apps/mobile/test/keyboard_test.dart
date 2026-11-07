import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/keyboard.dart';
import 'package:sanser_mobile/src/native_engine.dart';

class KeyboardEngine extends NativeEngine {
  final keys = <(int, bool)>[];
  final edits = <(String, String)>[];
  bool acceptText = true;
  @override
  Future<bool> text(String id, String text) async {
    edits.add((id, text));
    return acceptText;
  }

  @override
  Future<void> key(int code, bool down) async {
    keys.add((code, down));
  }
}

void main() {
  test('Unicode limit counts UTF16 units and preserves complete graphemes', () {
    expect(validKeyboardText('😀' * 128, unicodeSupported: true), isTrue);
    expect(validKeyboardText('😀' * 129, unicodeSupported: true), isFalse);
    expect(validKeyboardText('e\u0301' * 128, unicodeSupported: true), isTrue);
    expect(validKeyboardText('e\u0301' * 129, unicodeSupported: true), isFalse);
    expect(validKeyboardText('', unicodeSupported: true), isFalse);
    expect(validKeyboardText('a\u001b', unicodeSupported: true), isFalse);
  });
  test('old-host ASCII accepts literal backslash, Tab and newline', () async {
    final engine = KeyboardEngine();
    await sendText(engine, '\\\t\n');
    expect(engine.keys, [
      (49, true),
      (49, false),
      (43, true),
      (43, false),
      (40, true),
      (40, false),
    ]);
  });
  test('uppercase and punctuation release Shift after injection', () async {
    final e = KeyboardEngine();
    await sendText(e, 'A!a');
    expect(e.keys, [
      (225, true),
      (4, true),
      (4, false),
      (225, false),
      (225, true),
      (30, true),
      (30, false),
      (225, false),
      (4, true),
      (4, false),
    ]);
  });
  test('unsupported IME text never sends a partial prefix', () async {
    final e = KeyboardEngine();
    await expectLater(sendText(e, 'aế'), throwsArgumentError);
    expect(e.keys, isEmpty);
  });
  test(
    'negotiated Unicode sends one committed edit, preserving its session',
    () async {
      final e = KeyboardEngine();
      await sendText(
        e,
        'Tiếng Việt e\u0301 😀 中文',
        sessionId: 'session-a',
        unicodeSupported: true,
      );
      expect(e.edits, [('session-a', 'Tiếng Việt e\u0301 😀 中文')]);
      expect(e.keys, isEmpty);
    },
  );
  test(
    'invalid Unicode never sends a prefix or replacement characters',
    () async {
      final e = KeyboardEngine();
      for (final invalid in [
        '',
        'a\u0000b',
        'a\u001b',
        'a\ud800',
        'a\udc00',
        'a' * 257,
        '😀' * 129,
      ]) {
        await expectLater(
          sendText(e, invalid, sessionId: 'session-a', unicodeSupported: true),
          throwsArgumentError,
        );
      }
      expect(e.edits, isEmpty);
      expect(e.keys, isEmpty);
    },
  );
  test(
    'native rejection never falls back and duplicates part of a Unicode edit',
    () async {
      final e = KeyboardEngine()..acceptText = false;
      await expectLater(
        sendText(e, 'aế', sessionId: 'old-session', unicodeSupported: true),
        throwsStateError,
      );
      expect(e.edits, [('old-session', 'aế')]);
      expect(e.keys, isEmpty);
    },
  );
}
