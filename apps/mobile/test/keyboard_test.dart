import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/keyboard.dart';
import 'package:sanser_mobile/src/native_engine.dart';

class KeyboardEngine extends NativeEngine {
  final keys = <(int, bool)>[];
  @override
  Future<void> key(int code, bool down) async {
    keys.add((code, down));
  }
}

void main() {
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
}
