import 'native_engine.dart';

/// USB HID usages, shared with the desktop input engine. The first mobile
/// keyboard supports ASCII; it never pretends to inject unsupported IME text.
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

Future<void> sendText(NativeEngine engine, String text) async {
  if (text.length > 256 || text.split('').any((c) => hidFor(c) == null)) {
    throw ArgumentError(
      'Bàn phím mobile hiện hỗ trợ chữ Latin không dấu, số và ký hiệu ASCII.',
    );
  }
  for (final character in text.split('')) {
    final mapping = hidFor(character)!;
    try {
      if (mapping.$2) await engine.key(225, true);
      await engine.key(mapping.$1, true);
      await engine.key(mapping.$1, false);
    } finally {
      if (mapping.$2) await engine.key(225, false);
    }
  }
}
