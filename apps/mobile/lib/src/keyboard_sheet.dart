import 'package:flutter/material.dart';
import 'keyboard.dart';
import 'mobile_input.dart';

/// The phone owns the IME candidate UI and draft selection. Only an explicitly
/// confirmed, complete draft crosses the existing reliable Unicode input lane.
/// Never rewrite composing ranges in a controller listener (Gboard can restore
/// them), and never emulate candidate edits with remote backspaces.
class RemoteKeyboardSheet extends StatefulWidget {
  const RemoteKeyboardSheet({
    super.key,
    required this.input,
    required this.current,
    required this.unicodeSupported,
  });
  final MobileInput input;
  final bool Function() current;
  final bool unicodeSupported;
  @override
  State<RemoteKeyboardSheet> createState() => _RemoteKeyboardSheetState();
}

class _RemoteKeyboardSheetState extends State<RemoteKeyboardSheet> {
  final text = TextEditingController();
  final focus = FocusNode();
  bool busy = false;
  String message = '';
  int revision = 0;
  bool get composing =>
      text.value.composing.isValid && !text.value.composing.isCollapsed;

  @override
  void didUpdateWidget(covariant RemoteKeyboardSheet oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.input != widget.input) {
      revision++;
      busy = false;
      message = '';
      // Session changes must never inherit drafts or composition from a peer.
      focus.unfocus();
      text.clear();
    }
  }

  @override
  void dispose() {
    revision++;
    focus.dispose();
    text.dispose();
    super.dispose();
  }

  Future<void> run(Future<void> Function(MobileInput) action) async {
    if (busy || composing || !widget.current()) return;
    final input = widget.input, attempt = ++revision;
    bool current() =>
        mounted &&
        widget.input == input &&
        revision == attempt &&
        widget.current();
    setState(() {
      busy = true;
      message = '';
    });
    try {
      await input.drain();
      if (!current()) return;
      await action(input);
    } catch (_) {
      if (current()) {
        setState(() => message = 'Không gửi được phím. Kiểm tra kết nối.');
      }
    } finally {
      if (mounted && widget.input == input && revision == attempt) {
        setState(() => busy = false);
      }
    }
  }

  Future<void> send() async {
    if (busy || composing || !widget.current()) return;
    final snapshot = text.value;
    final unicode = widget.unicodeSupported;
    if (!validKeyboardText(snapshot.text, unicodeSupported: unicode)) return;
    final input = widget.input, attempt = ++revision;
    bool current() =>
        mounted &&
        widget.input == input &&
        revision == attempt &&
        widget.current();
    setState(() {
      busy = true;
      message = '';
    });
    try {
      // Unicode injection releases host modifiers. Reset local ownership too,
      // and order the edit after earlier remote key transitions.
      await input.releaseAll();
      await input.drain();
      if (!current()) return;
      await sendText(
        input.engine,
        snapshot.text,
        sessionId: input.sessionId,
        unicodeSupported: unicode,
      );
      if (mounted && current()) Navigator.pop(context);
    } catch (error) {
      // Keep the original draft/selection for review; no automatic retry after
      // an ambiguous transport error, since a prefix could have been injected.
      if (current()) setState(() => message = error.toString());
    } finally {
      if (mounted && widget.input == input && revision == attempt) {
        setState(() => busy = false);
      }
    }
  }

  @override
  Widget build(
    BuildContext context,
  ) => ValueListenableBuilder<TextEditingValue>(
    valueListenable: text,
    builder: (context, value, _) {
      final connected = widget.current();
      final canKey = connected && !busy && !composing;
      final valid = validKeyboardText(
        value.text,
        unicodeSupported: widget.unicodeSupported,
      );
      return SingleChildScrollView(
        child: Padding(
          padding: EdgeInsets.fromLTRB(
            20,
            24,
            20,
            MediaQuery.viewInsetsOf(context).bottom + 24,
          ),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              const Text(
                'Bàn phím từ xa',
                style: TextStyle(fontSize: 20, fontWeight: FontWeight.w600),
              ),
              const SizedBox(height: 16),
              TextField(
                controller: text,
                focusNode: focus,
                autofocus: true,
                readOnly: busy || !connected,
                minLines: 1,
                maxLines: 4,
                keyboardType: TextInputType.multiline,
                textInputAction: TextInputAction.done,
                // Finalize via the framework on explicit Done/Chốt chữ. Send
                // is a separate action after the final draft can be reviewed.
                onEditingComplete: focus.unfocus,
                onTapOutside: (_) {},
                decoration: InputDecoration(
                  labelText: widget.unicodeSupported
                      ? 'Nhập văn bản · tiếng Việt và emoji'
                      : 'Nhập văn bản · chữ không dấu',
                  helperText: composing
                      ? 'Đang ghép chữ · chọn gợi ý hoặc bấm Chốt chữ.'
                      : 'Soạn và chọn chữ trên điện thoại, rồi Gửi vào ô trên máy tính.',
                  helperMaxLines: 3,
                  counterText: '${value.text.length}/256 đơn vị Unicode',
                  errorText: value.text.isNotEmpty && !valid && !composing
                      ? value.text.length > 256
                            ? 'Văn bản quá dài. Chia thành đoạn ngắn hơn.'
                            : widget.unicodeSupported
                            ? 'Văn bản chứa ký tự không được hỗ trợ.'
                            : 'Host chưa hỗ trợ Unicode. Hãy cập nhật host.'
                      : null,
                ),
              ),
              const SizedBox(height: 12),
              if (composing)
                OutlinedButton.icon(
                  onPressed: busy || !connected ? null : focus.unfocus,
                  icon: const Icon(Icons.done),
                  label: const Text('Chốt chữ'),
                ),
              FilledButton(
                onPressed: canKey && valid ? send : null,
                child: Text(busy ? 'Đang gửi…' : 'Gửi'),
              ),
              if (!connected)
                const Text('Phiên điều khiển đã kết thúc. Bản nháp chưa gửi.'),
              if (message.isNotEmpty)
                Text(
                  message,
                  style: TextStyle(color: Theme.of(context).colorScheme.error),
                ),
              const SizedBox(height: 16),
              const Text('Phím gửi trực tiếp đến máy tính'),
              const SizedBox(height: 8),
              Wrap(
                spacing: 6,
                runSpacing: 6,
                children: [
                  for (final key in const [
                    (41, 'Esc'),
                    (43, 'Tab'),
                    (40, 'Enter'),
                    (42, '⌫'),
                    (82, '↑'),
                    (81, '↓'),
                    (80, '←'),
                    (79, '→'),
                  ])
                    OutlinedButton(
                      onPressed: canKey
                          ? () => run((input) => input.tapKey(key.$1))
                          : null,
                      child: Text(key.$2),
                    ),
                  for (var key = 58; key <= 69; key++)
                    OutlinedButton(
                      onPressed: canKey
                          ? () => run((input) => input.tapKey(key))
                          : null,
                      child: Text('F${key - 57}'),
                    ),
                  for (final chord in const [
                    (224, 6, 'Ctrl+C'),
                    (224, 25, 'Ctrl+V'),
                    (227, 6, '⌘C'),
                    (227, 25, '⌘V'),
                  ])
                    OutlinedButton(
                      onPressed: !canKey
                          ? null
                          : () => run((input) async {
                              final owner = Object();
                              try {
                                await input.hold(chord.$1, true, owner);
                                await input.tapKey(chord.$2);
                              } finally {
                                await input.hold(chord.$1, false, owner);
                              }
                            }),
                      child: Text(chord.$3),
                    ),
                ],
              ),
            ],
          ),
        ),
      );
    },
  );
}
