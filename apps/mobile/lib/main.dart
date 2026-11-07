import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'src/api.dart';
import 'src/controller.dart';
import 'src/native_engine.dart';
import 'src/screens.dart';
import 'src/theme.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  LicenseRegistry.addLicense(() async* {
    for (final entry in const {
      'Opus': 'opus.txt',
      'Tink / OkHttp (Apache-2.0)': 'apache-2.0.txt',
      'libsodium': 'libsodium.txt',
      'swift-sodium': 'swift-sodium.txt',
    }.entries) {
      yield LicenseEntryWithLineBreaks([
        entry.key,
      ], await rootBundle.loadString('assets/licenses/${entry.value}'));
    }
  });
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
    theme: sanserTheme(),
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
