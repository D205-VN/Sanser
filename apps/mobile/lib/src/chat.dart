import 'dart:convert';
import 'dart:math';

/// In-session chat, compatible with the desktop chat (`collab.rs`). Messages
/// live only in memory for the session and are always shown as plain text.
class ChatMessage {
  final String id, name, text;
  final bool own;
  final DateTime at;
  const ChatMessage({
    required this.id,
    required this.name,
    required this.text,
    required this.own,
    required this.at,
  });
}

class SessionChat {
  static const maxBytes = 1024, maxName = 64, history = 100;
  final List<ChatMessage> messages = [];
  final Random _random;
  SessionChat([Random? random]) : _random = random ?? Random.secure();

  /// Plain text, at most 1 KB, no control characters except newline and tab.
  static bool valid(String text) =>
      text.trim().isNotEmpty &&
      utf8.encode(text).length <= maxBytes &&
      !RegExp(
        r'[\u0000-\u0008\u000b-\u001f\u007f-\u009f]',
        unicode: true,
      ).hasMatch(text);

  static String cleanName(String name) {
    final kept = name.runes
        .where((r) => r >= 0x20 && !(r >= 0x7f && r <= 0x9f))
        .take(maxName);
    return String.fromCharCodes(kept).trim();
  }

  String _id() => List.generate(
    8,
    (_) => _random.nextInt(256).toRadixString(16).padLeft(2, '0'),
  ).join();

  /// The wire payload for a message composed on this phone, or null when the
  /// text is not allowed.
  ({String payload, ChatMessage message})? compose(String text, String name) {
    if (!valid(text)) return null;
    final id = _id(), sender = cleanName(name);
    return (
      payload: jsonEncode({
        'type': 'chat',
        'id': id,
        'name': sender,
        'text': text,
      }),
      message: ChatMessage(
        id: id,
        name: sender,
        text: text,
        own: true,
        at: DateTime.now(),
      ),
    );
  }

  void add(ChatMessage message) {
    messages.add(message);
    if (messages.length > history) messages.removeAt(0);
  }

  /// Parses a payload from the host. Drawing (stroke/clear) and policy
  /// records are not shown on phones and return null, as does anything
  /// malformed.
  ChatMessage? receive(String payload) {
    if (payload.isEmpty || payload.length > 4096) return null;
    Object? decoded;
    try {
      decoded = jsonDecode(payload);
    } on FormatException {
      return null;
    }
    if (decoded is! Map || decoded['type'] != 'chat') return null;
    final id = decoded['id'], text = decoded['text'], name = decoded['name'];
    if (id is! String ||
        id.isEmpty ||
        id.length > 32 ||
        !RegExp(r'^[A-Za-z0-9_-]+$').hasMatch(id) ||
        text is! String ||
        !valid(text)) {
      return null;
    }
    if (messages.any((m) => m.id == id && !m.own)) return null;
    final sender = name is String ? cleanName(name) : '';
    final message = ChatMessage(
      id: id,
      name: sender.isEmpty ? 'Máy tính' : sender,
      text: text,
      own: false,
      at: DateTime.now(),
    );
    add(message);
    return message;
  }

  void clear() => messages.clear();
}
