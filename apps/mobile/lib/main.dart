import 'package:flutter/material.dart';
import 'src/api.dart';
import 'src/controller.dart';
import 'src/native_engine.dart';
import 'src/screens.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  runApp(
    SanserApp(
      controller: MobileController(SanserApi(DeviceSecrets()), NativeEngine()),
    ),
  );
}

class SanserApp extends StatefulWidget {
  final MobileController controller;
  const SanserApp({super.key, required this.controller});
  @override
  State<SanserApp> createState() => _SanserAppState();
}

class _SanserAppState extends State<SanserApp> {
  late final Future<void> ready = widget.controller.initialize();
  @override
  Widget build(BuildContext context) => MaterialApp(
    title: 'Sanser',
    debugShowCheckedModeBanner: false,
    theme: ThemeData(
      brightness: Brightness.dark,
      useMaterial3: true,
      scaffoldBackgroundColor: const Color(0xff0e1119),
      colorScheme: ColorScheme.fromSeed(
        seedColor: const Color(0xffb9c8ff),
        brightness: Brightness.dark,
        primary: const Color(0xffb9c8ff),
        secondary: const Color(0xff78dfc0),
        surface: const Color(0xff191f2c),
      ),
      inputDecorationTheme: InputDecorationTheme(
        filled: true,
        fillColor: const Color(0xff101620),
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(16),
          borderSide: BorderSide.none,
        ),
      ),
      cardTheme: CardThemeData(
        elevation: 0,
        margin: EdgeInsets.zero,
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(22)),
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          minimumSize: const Size(48, 52),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(16),
          ),
        ),
      ),
    ),
    home: FutureBuilder<void>(
      future: ready,
      builder: (context, snapshot) =>
          snapshot.connectionState != ConnectionState.done
          ? const Scaffold(body: Center(child: CircularProgressIndicator()))
          : SanserHome(controller: widget.controller),
    ),
  );
  @override
  void dispose() {
    widget.controller.dispose();
    super.dispose();
  }
}
