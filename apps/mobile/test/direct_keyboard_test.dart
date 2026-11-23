import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sanser_mobile/src/direct_keyboard.dart';
import 'package:sanser_mobile/src/mobile_input.dart';
import 'keyboard_sheet_test.dart' show ImeEngine;

void main() {
  testWidgets('the phone keyboard types straight into the computer', (
    tester,
  ) async {
    final engine = ImeEngine();
    final input = MobileInput(engine, 'session-a', enabled: true);
    var closed = 0;
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Align(
            alignment: Alignment.bottomCenter,
            child: DirectKeyboard(
              input: input,
              unicodeSupported: true,
              current: () => true,
              onClose: () => closed++,
              onCompose: () {},
            ),
          ),
        ),
      ),
    );
    await tester.pump();
    // No visible text box: only the key row.
    expect(find.text('Esc'), findsOneWidget);
    const guard = DirectKeyboardState.guard;
    Future<void> type(String value) async {
      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: value,
          selection: TextSelection.collapsed(offset: value.length),
        ),
      );
      await tester.pump();
      await input.drain();
      await tester.pump();
    }

    await type('${guard}tìm');
    await type('${guard}tìm k');
    expect(engine.edits.map((e) => e.$2), ['tìm', ' k']);
    // Backspace past the start deletes text that was already on the computer.
    await type(guard);
    await type(guard.substring(1));
    final backspaces = engine.keys.where((k) => k.$2 == 42 && k.$3).length;
    expect(backspaces, 6, reason: '5 typed characters, then 1 older one');
    await tester.testTextInput.receiveAction(TextInputAction.go);
    await tester.pump();
    await input.drain();
    await tester.pump();
    expect(engine.keys.where((k) => k.$2 == 40 && k.$3), hasLength(1));
    await tester.tap(find.byKey(const ValueKey('direct-keyboard-close')));
    expect(closed, 1);
    input.dispose();
    await input.drain();
  });
}
