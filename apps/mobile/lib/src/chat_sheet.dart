import 'dart:async';
import 'package:flutter/material.dart';
import 'chat.dart';
import 'controller.dart';
import 'design_tokens.dart';

/// In-session chat with the computer (and, in sharing sessions, the other
/// viewers). Text only; nothing is saved after the session ends.
class ChatSheet extends StatefulWidget {
  final MobileController controller;
  const ChatSheet({super.key, required this.controller});
  @override
  State<ChatSheet> createState() => _ChatSheetState();
}

class _ChatSheetState extends State<ChatSheet> {
  final text = TextEditingController();
  final scroll = ScrollController();
  bool sending = false;
  String? problem;
  MobileController get c => widget.controller;

  @override
  void initState() {
    super.initState();
    // Not during build: opening clears the unread badge on the session screen.
    scheduleMicrotask(() {
      if (mounted) c.setChatOpen(true);
    });
    c.addListener(_changed);
    text.addListener(() => setState(() {}));
  }

  void _changed() {
    if (!mounted) return;
    setState(() {});
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (scroll.hasClients) scroll.jumpTo(scroll.position.maxScrollExtent);
    });
  }

  @override
  void dispose() {
    c.removeListener(_changed);
    scheduleMicrotask(() => c.setChatOpen(false));
    text.dispose();
    scroll.dispose();
    super.dispose();
  }

  Future<void> _send() async {
    final value = text.text;
    if (sending || !SessionChat.valid(value)) return;
    setState(() {
      sending = true;
      problem = null;
    });
    final sent = await c.sendChat(value);
    if (!mounted) return;
    setState(() {
      sending = false;
      if (sent) {
        text.clear();
      } else {
        problem = c.chatAvailable
            ? 'Chưa gửi được. Bạn gửi hơi nhanh, thử lại sau giây lát.'
            : 'Máy tính chưa hỗ trợ chat hoặc kết nối chưa sẵn sàng.';
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final messages = c.chat.messages;
    final tooLong =
        !SessionChat.valid(text.text) && text.text.trim().isNotEmpty;
    return Padding(
      padding: EdgeInsets.only(bottom: MediaQuery.viewInsetsOf(context).bottom),
      child: SizedBox(
        height: MediaQuery.sizeOf(context).height * 0.6,
        child: Column(
          children: [
            const Padding(
              padding: EdgeInsets.fromLTRB(20, 4, 20, 8),
              child: Row(
                children: [
                  Icon(Icons.chat_bubble_outline_rounded, size: 20),
                  SizedBox(width: 10),
                  Text(
                    'Chat trong phiên',
                    style: TextStyle(fontWeight: FontWeight.w600, fontSize: 16),
                  ),
                  Spacer(),
                  Text(
                    'Không lưu sau khi ngắt',
                    style: TextStyle(fontSize: 12, color: SanserTokens.muted),
                  ),
                ],
              ),
            ),
            const Divider(height: 1),
            Expanded(
              child: messages.isEmpty
                  ? const Center(
                      child: Text(
                        'Chưa có tin nhắn.',
                        style: TextStyle(color: SanserTokens.muted),
                      ),
                    )
                  : ListView.builder(
                      controller: scroll,
                      padding: const EdgeInsets.all(14),
                      itemCount: messages.length,
                      itemBuilder: (context, index) => _Bubble(messages[index]),
                    ),
            ),
            if (problem != null || tooLong)
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 16),
                child: Text(
                  tooLong ? 'Tin nhắn tối đa 1 KB, chỉ văn bản.' : problem!,
                  style: const TextStyle(
                    color: SanserTokens.warning,
                    fontSize: 12,
                  ),
                ),
              ),
            SafeArea(
              top: false,
              child: Padding(
                padding: const EdgeInsets.fromLTRB(12, 8, 8, 8),
                child: Row(
                  children: [
                    Expanded(
                      child: TextField(
                        key: const ValueKey('chat-input'),
                        controller: text,
                        enabled: c.chatAvailable,
                        minLines: 1,
                        maxLines: 4,
                        textInputAction: TextInputAction.send,
                        onSubmitted: (_) => _send(),
                        decoration: InputDecoration(
                          hintText: c.chatAvailable
                              ? 'Nhắn cho máy tính…'
                              : 'Máy tính chưa hỗ trợ chat',
                          isDense: true,
                        ),
                      ),
                    ),
                    IconButton(
                      key: const ValueKey('chat-send'),
                      tooltip: 'Gửi',
                      onPressed:
                          c.chatAvailable &&
                              !sending &&
                              SessionChat.valid(text.text)
                          ? _send
                          : null,
                      icon: const Icon(Icons.send_rounded),
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

class _Bubble extends StatelessWidget {
  final ChatMessage message;
  const _Bubble(this.message);
  @override
  Widget build(BuildContext context) {
    final time =
        '${message.at.hour.toString().padLeft(2, '0')}:${message.at.minute.toString().padLeft(2, '0')}';
    return Align(
      alignment: message.own ? Alignment.centerRight : Alignment.centerLeft,
      child: Container(
        margin: const EdgeInsets.symmetric(vertical: 4),
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
        constraints: BoxConstraints(
          maxWidth: MediaQuery.sizeOf(context).width * 0.75,
        ),
        decoration: BoxDecoration(
          color: message.own ? SanserTokens.selected : SanserTokens.surface,
          borderRadius: BorderRadius.circular(14),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              message.own ? 'Bạn · $time' : '${message.name} · $time',
              style: const TextStyle(fontSize: 11, color: SanserTokens.muted),
            ),
            const SizedBox(height: 2),
            // Plain text only: never interpreted as markup or links.
            SelectableText(message.text),
          ],
        ),
      ),
    );
  }
}
