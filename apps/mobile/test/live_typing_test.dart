import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/keyboard.dart';
import 'keyboard_sheet_test.dart' show ImeEngine, open, edit;

void main() {
  test('live edits are the smallest Backspace + text change', () {
    expect(liveEdit('', 'abc'), (0, 'abc'));
    expect(liveEdit('abc', 'ab'), (1, ''));
    expect(liveEdit('teh ', 'the '), (3, 'he '));
    expect(liveEdit('a😀', 'a'), (1, ''), reason: 'an emoji is one Backspace');
    expect(
      committedText(
        const TextEditingValue(
          text: 'xin chaa',
          composing: TextRange(start: 6, end: 8),
        ),
      ),
      'xin ch',
    );
  });

  testWidgets('typing appears on the computer without pressing Send', (
    tester,
  ) async {
    final engine = ImeEngine();
    final input = await open(tester, engine);
    expect(find.text('Gửi'), findsNothing);
    await edit(tester, 'xin');
    await edit(tester, 'xin ch');
    // Telex still composing "â": nothing is sent for it yet.
    await edit(
      tester,
      'xin chaa',
      composing: const TextRange(start: 6, end: 8),
    );
    await edit(tester, 'xin châ');
    await tester.pump();
    await input.drain();
    await tester.pump();
    expect(engine.edits.map((e) => e.$2), ['xin', ' ch', 'â']);
    // Deleting on the phone deletes on the computer.
    await edit(tester, 'xin ch');
    await tester.pump();
    await input.drain();
    await tester.pump();
    final backspaces = engine.keys.where((k) => k.$2 == 42 && k.$3);
    expect(backspaces, hasLength(1));
    // A new line is the Enter key.
    await edit(tester, 'xin ch\n');
    await tester.pump();
    await input.drain();
    await tester.pump();
    expect(engine.keys.where((k) => k.$2 == 40 && k.$3), hasLength(1));
    input.dispose();
    await input.drain();
  });
}
