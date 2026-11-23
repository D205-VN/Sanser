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
    final size = MediaQuery.sizeOf(context);
    final keyboard = MediaQuery.viewInsetsOf(context).bottom;
    return Padding(
      padding: EdgeInsets.only(bottom: keyboard),
      child: SizedBox(
        // Tall enough to read a conversation; with the keyboard up the
        // sheet shrinks so the newest messages stay above it.
        height: keyboard > 0
            ? (size.height - keyboard) * 0.85
            : size.height * 0.8,
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(20, 0, 12, 10),
              child: Row(
                children: [
                  const Icon(
                    Icons.chat_bubble_rounded,
                    size: 22,
                    color: SanserTokens.accent,
                  ),
                  const SizedBox(width: 10),
                  const Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          'Chat',
                          style: TextStyle(
                            fontWeight: FontWeight.w700,
                            fontSize: 18,
                          ),
                        ),
                        Text(
                          'Không lưu sau khi ngắt kết nối',
                          style: TextStyle(
                            fontSize: 12,
                            color: SanserTokens.muted,
                          ),
                        ),
                      ],
                    ),
                  ),
                  IconButton(
                    key: const ValueKey('chat-close'),
                    tooltip: 'Đóng chat',
                    onPressed: () => Navigator.of(context).maybePop(),
                    icon: const Icon(Icons.close_rounded),
                  ),
                ],
              ),
            ),
            const Divider(height: 1),
            Expanded(
              child: messages.isEmpty
                  ? const Center(
                      child: Padding(
                        padding: EdgeInsets.all(24),
                        child: Column(
                          mainAxisSize: MainAxisSize.min,
                          children: [
                            Icon(
                              Icons.forum_outlined,
                              size: 40,
                              color: SanserTokens.mutedLow,
                            ),
                            SizedBox(height: 12),
                            Text(
                              'Chưa có tin nhắn.\nNhắn gì đó cho máy tính bên kia.',
                              textAlign: TextAlign.center,
                              style: TextStyle(
                                color: SanserTokens.muted,
                                fontSize: 15,
                                height: 1.4,
                              ),
                            ),
                          ],
                        ),
                      ),
                    )
                  : ListView.builder(
                      controller: scroll,
                      padding: const EdgeInsets.fromLTRB(14, 14, 14, 8),
                      itemCount: messages.length,
                      itemBuilder: (context, index) => _Bubble(
                        messages[index],
                        // Group a run from one sender under one name.
                        showName:
                            !messages[index].own &&
                            (index == 0 ||
                                messages[index - 1].own ||
                                messages[index - 1].name !=
                                    messages[index].name),
                      ),
                    ),
            ),
            if (problem != null || tooLong)
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 4, 16, 0),
                child: Text(
                  tooLong ? 'Tin nhắn tối đa 1 KB, chỉ văn bản.' : problem!,
                  style: const TextStyle(
                    color: SanserTokens.warning,
                    fontSize: 13,
                  ),
                ),
              ),
            SafeArea(
              top: false,
              child: Padding(
                padding: const EdgeInsets.fromLTRB(12, 8, 12, 10),
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.end,
                  children: [
                    Expanded(
                      child: TextField(
                        key: const ValueKey('chat-input'),
                        controller: text,
                        enabled: c.chatAvailable,
                        autofocus: messages.isEmpty,
                        minLines: 1,
                        maxLines: 4,
                        textCapitalization: TextCapitalization.sentences,
                        textInputAction: TextInputAction.send,
                        onSubmitted: (_) => _send(),
                        style: const TextStyle(fontSize: 16),
                        decoration: InputDecoration(
                          hintText: c.chatAvailable
                              ? 'Nhắn tin…'
                              : 'Máy tính chưa hỗ trợ chat',
                          filled: true,
                          fillColor: SanserTokens.raised,
                          contentPadding: const EdgeInsets.symmetric(
                            horizontal: 16,
                            vertical: 12,
                          ),
                          border: OutlineInputBorder(
                            borderRadius: BorderRadius.circular(22),
                            borderSide: BorderSide.none,
                          ),
                        ),
                      ),
                    ),
                    const SizedBox(width: 8),
                    IconButton.filled(
                      key: const ValueKey('chat-send'),
                      tooltip: 'Gửi',
                      style: IconButton.styleFrom(
                        minimumSize: const Size(46, 46),
                        backgroundColor: const Color(0xff4f8bff),
                        foregroundColor: Colors.white,
                      ),
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

String _clock(DateTime at) =>
    '${at.hour.toString().padLeft(2, '0')}:${at.minute.toString().padLeft(2, '0')}';

class _Bubble extends StatelessWidget {
  final ChatMessage message;
  final bool showName;
  const _Bubble(this.message, {this.showName = false});
  @override
  Widget build(BuildContext context) {
    final own = message.own;
    return Padding(
      padding: EdgeInsets.only(top: showName ? 10 : 3, bottom: 3),
      child: Column(
        crossAxisAlignment: own
            ? CrossAxisAlignment.end
            : CrossAxisAlignment.start,
        children: [
          if (showName)
            Padding(
              padding: const EdgeInsets.only(left: 12, bottom: 4),
              child: Text(
                message.name,
                style: const TextStyle(
                  fontSize: 13,
                  fontWeight: FontWeight.w600,
                  color: SanserTokens.accent,
                ),
              ),
            ),
          Container(
            padding: const EdgeInsets.fromLTRB(14, 10, 14, 8),
            constraints: BoxConstraints(
              maxWidth: MediaQuery.sizeOf(context).width * 0.78,
            ),
            decoration: BoxDecoration(
              color: own ? null : SanserTokens.raised,
              gradient: own
                  ? const LinearGradient(
                      begin: Alignment.topLeft,
                      end: Alignment.bottomRight,
                      colors: [Color(0xff4f8bff), Color(0xff8b5cff)],
                    )
                  : null,
              borderRadius: BorderRadius.only(
                topLeft: const Radius.circular(18),
                topRight: const Radius.circular(18),
                bottomLeft: Radius.circular(own ? 18 : 6),
                bottomRight: Radius.circular(own ? 6 : 18),
              ),
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: [
                // Plain text only: never interpreted as markup or links.
                SelectableText(
                  message.text,
                  style: TextStyle(
                    fontSize: 16,
                    height: 1.35,
                    color: own ? Colors.white : SanserTokens.text,
                  ),
                ),
                const SizedBox(height: 3),
                Text(
                  _clock(message.at),
                  style: TextStyle(
                    fontSize: 11,
                    color: own
                        ? Colors.white.withValues(alpha: 0.7)
                        : SanserTokens.mutedLow,
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
