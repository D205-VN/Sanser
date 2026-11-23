import 'dart:async';
import 'package:flutter/material.dart';
import 'keyboard.dart';
import 'mobile_input.dart';

/// Sends live edits in order: Backspaces, text, and Enter for new lines.
class LiveTyper {
  LiveTyper(this.input, {required this.unicode, required this.current});
  final MobileInput input;
  final bool unicode;
  final bool Function() current;
  String sent = '';
  Future<void> _pending = Future.value();
  void Function(String)? onError;

  void _queue(Future<void> Function() work) {
    _pending = _pending.then((_) async {
      if (!current()) return;
      try {
        await work();
      } catch (error) {
        onError?.call(error.toString());
      }
    });
  }

  /// Make the computer match [committed], the phone's finished text.
  void update(String committed) {
    if (committed == sent) return;
    final (erase, insert) = liveEdit(sent, committed);
    sent = committed;
    _queue(() async {
      for (var i = 0; i < erase; i++) {
        await input.tapKey(42);
      }
      final lines = insert.split('\n');
      for (var i = 0; i < lines.length; i++) {
        if (i > 0) await input.tapKey(40);
        if (lines[i].isEmpty) continue;
        await input.drain();
        await sendText(
          input.engine,
          lines[i],
          sessionId: input.sessionId,
          unicodeSupported: unicode,
        );
      }
    });
  }

  /// Backspaces past what this box typed (deleting older text on the host).
  void backspace(int count) => _queue(() async {
    for (var i = 0; i < count; i++) {
      await input.tapKey(42);
    }
  });

  void key(int code) => _queue(() => input.tapKey(code));

  /// Forget the box's text (the host keeps what was typed).
  void restart() => sent = '';

  Future<void> drain() => _pending;
}

/// The phone's own keyboard typing straight into the computer: no text box,
/// just the system keyboard and a slim row of keys it lacks.
class DirectKeyboard extends StatefulWidget {
  const DirectKeyboard({
    super.key,
    required this.input,
    required this.unicodeSupported,
    required this.current,
    required this.onClose,
    required this.onCompose,
  });
  final MobileInput input;
  final bool unicodeSupported;
  final bool Function() current;
  final VoidCallback onClose;
  final VoidCallback onCompose;
  @override
  State<DirectKeyboard> createState() => DirectKeyboardState();
}

class DirectKeyboardState extends State<DirectKeyboard> {
  /// Invisible text the keyboard can delete, so Backspace on an "empty"
  /// box still reaches the computer.
  static const guard = '​​​​';
  final focus = FocusNode();
  late final text = TextEditingController(text: guard);
  late LiveTyper typer;
  String message = '';

  @override
  void initState() {
    super.initState();
    typer = _typer();
    text.addListener(_changed);
  }

  LiveTyper _typer() =>
      LiveTyper(
          widget.input,
          unicode: widget.unicodeSupported,
          current: () => mounted && widget.current(),
        )
        ..onError = (error) {
          if (mounted) setState(() => message = error);
        };

  @override
  void didUpdateWidget(covariant DirectKeyboard oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.input != widget.input ||
        oldWidget.unicodeSupported != widget.unicodeSupported) {
      typer = _typer();
      _reset();
    }
  }

  @override
  void dispose() {
    text.removeListener(_changed);
    text.dispose();
    focus.dispose();
    super.dispose();
  }

  void _reset() {
    typer.restart();
    text.value = const TextEditingValue(
      text: guard,
      selection: TextSelection.collapsed(offset: guard.length),
    );
  }

  void _changed() {
    if (!widget.current()) return;
    final value = text.value;
    if (!value.text.startsWith(guard)) {
      // Deleted into the guard: each guard character is one Backspace on
      // the computer, deleting text typed before the keyboard opened.
      final rest = value.text.replaceFirst(RegExp('^\u200B*'), '');
      final erased = guard.length - (value.text.length - rest.length);
      typer.update('');
      if (erased > 0) typer.backspace(erased);
      typer.restart();
      text.value = TextEditingValue(
        text: guard + rest,
        selection: TextSelection.collapsed(offset: guard.length + rest.length),
      );
      return;
    }
    final committed = committedText(value).substring(guard.length);
    typer.update(committed);
    if (message.isNotEmpty) setState(() => message = '');
    // Keep the hidden box short; it never needs Backspaces to catch up.
    if (committed.length > 120 && !value.composing.isValid) _reset();
  }

  @override
  Widget build(BuildContext context) {
    final scheme = Theme.of(context).colorScheme;
    Widget key(String label, int code, {String? tooltip}) => Padding(
      padding: const EdgeInsets.symmetric(horizontal: 3),
      child: OutlinedButton(
        style: OutlinedButton.styleFrom(
          minimumSize: const Size(44, 36),
          padding: const EdgeInsets.symmetric(horizontal: 10),
          visualDensity: VisualDensity.compact,
        ),
        onPressed: () => typer.key(code),
        child: Text(label, semanticsLabel: tooltip),
      ),
    );
    return Material(
      key: const ValueKey('direct-keyboard'),
      color: scheme.surfaceContainerHigh,
      child: SafeArea(
        top: false,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            // Off-screen box that receives the system keyboard.
            SizedBox(
              height: 1,
              child: Opacity(
                opacity: 0,
                child: TextField(
                  key: const ValueKey('direct-keyboard-input'),
                  controller: text,
                  focusNode: focus,
                  autofocus: true,
                  autocorrect: false,
                  showCursor: false,
                  enableInteractiveSelection: false,
                  textInputAction: TextInputAction.go,
                  onEditingComplete: () {},
                  onSubmitted: (_) {
                    typer.key(40);
                    focus.requestFocus();
                  },
                  decoration: const InputDecoration.collapsed(hintText: ''),
                ),
              ),
            ),
            if (message.isNotEmpty)
              Padding(
                padding: const EdgeInsets.fromLTRB(12, 6, 12, 0),
                child: Text(message, style: TextStyle(color: scheme.error)),
              ),
            SizedBox(
              height: 48,
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 6),
                child: Row(
                  children: [
                    IconButton(
                      key: const ValueKey('direct-keyboard-close'),
                      tooltip: 'Ẩn bàn phím',
                      onPressed: widget.onClose,
                      icon: const Icon(Icons.keyboard_hide_outlined),
                    ),
                    key('Esc', 41),
                    key('Tab', 43),
                    key('⌫', 42, tooltip: 'Xoá lùi'),
                    key('↵', 40, tooltip: 'Enter'),
                    key('←', 80),
                    key('↑', 82),
                    key('↓', 81),
                    key('→', 79),
                    Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 3),
                      child: TextButton(
                        onPressed: widget.onCompose,
                        child: const Text('Soạn rồi gửi'),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
