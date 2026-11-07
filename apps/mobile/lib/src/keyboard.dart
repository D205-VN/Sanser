import 'native_engine.dart';

/// USB HID usages, shared with the desktop input engine. The first mobile
/// keyboard uses physical HID keys for shortcuts and older ASCII-only hosts.
(int, bool)? hidFor(String character) {
  final code = character.codeUnitAt(0);
  if (code >= 97 && code <= 122) return (4 + code - 97, false);
  if (code >= 65 && code <= 90) return (4 + code - 65, true);
  if (code >= 49 && code <= 57) return (30 + code - 49, false);
  if (character == '0') return (39, false);
  const regular = {
    ' ': 44,
    '-': 45,
    '=': 46,
    '[': 47,
    ']': 48,
    '\\': 49,
    ';': 51,
    "'": 52,
    '`': 53,
    ',': 54,
    '.': 55,
    '/': 56,
    '\n': 40,
    '\t': 43,
  };
  const shifted = {
    '!': 30,
    '@': 31,
    '#': 32,
    r'$': 33,
    '%': 34,
    '^': 35,
    '&': 36,
    '*': 37,
    '(': 38,
    ')': 39,
    '_': 45,
    '+': 46,
    '{': 47,
    '}': 48,
    '|': 49,
    ':': 51,
    '"': 52,
    '~': 53,
    '<': 54,
    '>': 55,
    '?': 56,
  };
  if (regular.containsKey(character)) return (regular[character]!, false);
  if (shifted.containsKey(character)) return (shifted[character]!, true);
  return null;
}

Future<void> sendText(
  NativeEngine engine,
  String text, {
  String? sessionId,
  bool unicodeSupported = false,
}) async {
  if (unicodeSupported) {
    if (sessionId == null || !validKeyboardText(text, unicodeSupported: true)) {
      throw ArgumentError(
        'Nội dung quá dài hoặc chứa ký tự điều khiển không được hỗ trợ.',
      );
    }
    if (!await engine.text(sessionId, text)) {
      throw StateError(
        'Chưa gửi được chữ: kiểm tra phiên điều khiển và quyền nhập trên host.',
      );
    }
    return;
  }
  if (!validKeyboardText(text, unicodeSupported: false)) {
    throw ArgumentError(
      'Host chưa hỗ trợ nhập Unicode. Hãy cập nhật host; hiện chỉ gửi được chữ không dấu và ký hiệu ASCII.',
    );
  }
  Future<void> key(int code, bool down) => sessionId == null
      ? engine.key(code, down)
      : engine.remoteInput(sessionId, 2, code: code, down: down);
  for (final character in text.split('')) {
    final mapping = hidFor(character)!;
    try {
      if (mapping.$2) await key(225, true);
      await key(mapping.$1, true);
      await key(mapping.$1, false);
    } finally {
      if (mapping.$2) await key(225, false);
    }
  }
}

/// Match the native limit in UTF-16 units, not Flutter's grapheme counter. Do
/// not trim, normalize or truncate an IME draft: combining marks and emoji must
/// arrive exactly as the user confirmed them.
bool validKeyboardText(String text, {required bool unicodeSupported}) =>
    text.isNotEmpty &&
    text.length <= 256 &&
    (unicodeSupported
        ? _validText(text)
        : text.split('').every((c) => hidFor(c) != null));

bool _validText(String text) {
  for (var i = 0; i < text.length; ++i) {
    final unit = text.codeUnitAt(i);
    if ((unit < 32 && unit != 10 && unit != 9) ||
        (unit >= 127 && unit <= 159)) {
      return false;
    }
    if (unit >= 0xd800 && unit <= 0xdbff) {
      if (++i >= text.length) return false;
      final low = text.codeUnitAt(i);
      if (low < 0xdc00 || low > 0xdfff) return false;
    } else if (unit >= 0xdc00 && unit <= 0xdfff) {
      return false;
    }
  }
  return true;
}
