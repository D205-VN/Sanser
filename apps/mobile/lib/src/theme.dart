import 'package:flutter/material.dart';
import 'design_tokens.dart';

/// Second stop of the brand gradient; the first is [SanserTokens.accent].
const sanserViolet = Color(0xffb59cff);
const sanserAccentGradient = LinearGradient(
  begin: Alignment.topLeft,
  end: Alignment.bottomRight,
  colors: [SanserTokens.accent, sanserViolet],
);

ThemeData sanserTheme() {
  const t = 14.0;
  final outline = OutlineInputBorder(
    borderRadius: BorderRadius.circular(t),
    borderSide: const BorderSide(color: SanserTokens.borderStrong),
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
      titleTextStyle: TextStyle(
        fontSize: 22,
        fontWeight: FontWeight.w800,
        color: SanserTokens.text,
      ),
    ),
    textTheme: const TextTheme(
      headlineMedium: TextStyle(
        fontSize: 28,
        fontWeight: FontWeight.w700,
        letterSpacing: -.6,
        color: SanserTokens.text,
      ),
      titleLarge: TextStyle(
        fontSize: 20,
        fontWeight: FontWeight.w700,
        letterSpacing: -.3,
        color: SanserTokens.text,
      ),
      bodyMedium: TextStyle(
        fontSize: 15,
        height: 1.45,
        color: SanserTokens.text,
      ),
    ),
    dividerTheme: const DividerThemeData(
      color: SanserTokens.border,
      thickness: 1,
      space: 1,
    ),
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: SanserTokens.raised,
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
        borderRadius: BorderRadius.circular(20),
        side: const BorderSide(color: SanserTokens.border),
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        minimumSize: const Size(48, SanserTokens.touchTarget),
        textStyle: const TextStyle(fontSize: 15, fontWeight: FontWeight.w700),
        disabledBackgroundColor: SanserTokens.raised,
        disabledForegroundColor: SanserTokens.mutedLow,
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
      indicatorColor: SanserTokens.accent.withValues(alpha: .18),
      indicatorShape: const StadiumBorder(),
      height: 72,
      labelTextStyle: WidgetStateProperty.resolveWith(
        (states) => TextStyle(
          fontSize: 12,
          fontWeight: FontWeight.w600,
          color: states.contains(WidgetState.selected)
              ? SanserTokens.accent
              : SanserTokens.muted,
        ),
      ),
      iconTheme: WidgetStateProperty.resolveWith(
        (states) => IconThemeData(
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
        borderRadius: BorderRadius.vertical(top: Radius.circular(24)),
        side: const BorderSide(color: SanserTokens.border),
      ),
    ),
  );
}
