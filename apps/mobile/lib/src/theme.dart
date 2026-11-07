import 'package:flutter/material.dart';
import 'design_tokens.dart';

ThemeData sanserTheme() {
  const t = SanserTokens.controlRadius;
  final outline = OutlineInputBorder(
    borderRadius: BorderRadius.circular(t),
    borderSide: const BorderSide(color: SanserTokens.border),
  );
  return ThemeData(
    brightness: Brightness.dark,
    useMaterial3: true,
    scaffoldBackgroundColor: SanserTokens.background,
    colorScheme: const ColorScheme.dark(
      primary: SanserTokens.accent,
      onPrimary: SanserTokens.onAccent,
      secondary: SanserTokens.success,
      secondaryContainer: SanserTokens.selected,
      onSecondaryContainer: SanserTokens.accent,
      surface: SanserTokens.surface,
      onSurface: SanserTokens.text,
      onSurfaceVariant: SanserTokens.muted,
      error: SanserTokens.danger,
      outline: SanserTokens.borderStrong,
      outlineVariant: SanserTokens.border,
    ),
    appBarTheme: const AppBarTheme(
      backgroundColor: SanserTokens.background,
      foregroundColor: SanserTokens.text,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
      scrolledUnderElevation: 0,
    ),
    dividerTheme: const DividerThemeData(
      color: SanserTokens.border,
      thickness: 1,
      space: 1,
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: SanserTokens.background,
      contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 16),
      border: outline,
      enabledBorder: outline,
      focusedBorder: outline.copyWith(
        borderSide: const BorderSide(color: SanserTokens.accent, width: 2),
      ),
    ),
    cardTheme: CardThemeData(
      color: SanserTokens.surface,
      surfaceTintColor: Colors.transparent,
      elevation: 0,
      margin: EdgeInsets.zero,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.circular(SanserTokens.cardRadius),
        side: const BorderSide(color: SanserTokens.border),
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        minimumSize: const Size(48, SanserTokens.touchTarget),
        textStyle: const TextStyle(fontSize: 14, fontWeight: FontWeight.w600),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(t)),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        minimumSize: const Size(44, SanserTokens.touchTarget),
        side: const BorderSide(color: SanserTokens.borderStrong),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(t)),
      ),
    ),
    iconButtonTheme: IconButtonThemeData(
      style: IconButton.styleFrom(
        minimumSize: const Size.square(SanserTokens.touchTarget),
        foregroundColor: SanserTokens.textSoft,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(t)),
      ),
    ),
    navigationBarTheme: NavigationBarThemeData(
      backgroundColor: SanserTokens.background,
      surfaceTintColor: Colors.transparent,
      indicatorColor: SanserTokens.selected,
      height: 72,
      labelTextStyle: WidgetStateProperty.resolveWith(
        (states) => TextStyle(
          fontSize: 11,
          fontWeight: FontWeight.w600,
          color: states.contains(WidgetState.selected)
              ? SanserTokens.accent
              : SanserTokens.muted,
        ),
      ),
    ),
    bottomSheetTheme: BottomSheetThemeData(
      backgroundColor: SanserTokens.surface,
      surfaceTintColor: Colors.transparent,
      showDragHandle: true,
      shape: RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(
          top: Radius.circular(SanserTokens.cardRadius),
        ),
        side: const BorderSide(color: SanserTokens.border),
      ),
    ),
  );
}
