import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'api.dart';
import 'controller.dart';
import 'keyboard_sheet.dart';
import 'floating_controls.dart';
import 'settings.dart';
import 'mobile_input.dart';
import 'touch_surface.dart';
import 'design_tokens.dart';
import 'session_actions.dart';
import 'clipboard_sheet.dart';
import 'files.dart';
import 'external_keyboard.dart';
import 'haptics.dart';
import 'adaptive_layout.dart';
import 'gamepad_keys.dart';
import 'host_shortcuts.dart';

class SanserHome extends StatefulWidget {
  final MobileController controller;
  const SanserHome({super.key, required this.controller});
  @override
  State<SanserHome> createState() => _SanserHomeState();
}

class _SanserHomeState extends State<SanserHome> with WidgetsBindingObserver {
  int tab = 0;
  Timer? documentBackground;
  MobileController get c => widget.controller;
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    c.setForeground(
      WidgetsBinding.instance.lifecycleState == null ||
          WidgetsBinding.instance.lifecycleState == AppLifecycleState.resumed,
    );
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    c.setForeground(state == AppLifecycleState.resumed);
    if (state == AppLifecycleState.resumed) {
      documentBackground?.cancel();
      documentBackground = null;
    }
    if (state == AppLifecycleState.paused ||
        state == AppLifecycleState.detached) {
      if (state != AppLifecycleState.detached && c.engine.documentUIActive) {
        unawaited(c.engine.resetInput().catchError((Object _) {}));
        documentBackground?.cancel();
        documentBackground = Timer(const Duration(minutes: 2), () {
          if (mounted &&
              WidgetsBinding.instance.lifecycleState !=
                  AppLifecycleState.resumed) {
            unawaited(c.disconnect());
          }
        });
      } else {
        unawaited(c.disconnect());
      }
    }
    if (state == AppLifecycleState.inactive) {
      unawaited(c.engine.resetInput().catchError((Object _) {}));
    }
  }

  @override
  void dispose() {
    documentBackground?.cancel();
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: c,
    builder: (context, _) {
      if (c.account == null) return SignIn(controller: c);
      if (c.active != null) {
        return RemoteSession(
          key: ValueKey('${c.active!['id']}:${c.mediaRevision}'),
          controller: c,
        );
      }
      return MobileWorkspace(
        selectedIndex: tab,
        onSelected: (value) => setState(() => tab = value),
        appBar: AppBar(
          title: const Brand(),
          actions: [
            IconButton(
              tooltip: 'Làm mới',
              onPressed: () => c.refresh(),
              icon: const Icon(Icons.refresh_rounded),
            ),
            const SizedBox(width: 8),
          ],
        ),
        body: SafeArea(
          child: ListView(
            padding: const EdgeInsets.fromLTRB(22, 16, 22, 28),
            children: [
              if (c.error != null) ErrorNote(c.error!),
              Text(
                [
                  'Máy tính của bạn',
                  'Lời mời chia sẻ',
                  'Cài đặt',
                  'Tài khoản',
                ][tab],
                style: Theme.of(context).textTheme.headlineMedium?.copyWith(
                  fontWeight: FontWeight.w700,
                ),
              ),
              const SizedBox(height: 8),
              Text(
                [
                  'Kết nối và mang máy tính theo bên mình.',
                  'Xem màn hình được chia sẻ với bạn.',
                  'Điều chỉnh trải nghiệm trên điện thoại.',
                  'Sanser trên điện thoại của bạn.',
                ][tab],
                style: const TextStyle(color: Colors.white60),
              ),
              const SizedBox(height: 24),
              if (tab == 0) ...[
                if (c.capabilities['available'] != true)
                  const ErrorNote(
                    'Bản build này chưa có engine truyền hình native. Bạn vẫn có thể đăng nhập và xem danh sách máy.',
                  ),
                if (c.devices.isEmpty)
                  const EmptyCard(
                    icon: Icons.computer_rounded,
                    title: 'Chưa có máy tính',
                    message:
                        'Mở Sanser trên máy tính, đăng nhập cùng tài khoản rồi bật Host.',
                  ),
                MobileDeviceCards(
                  children: [
                    for (final device in c.devices)
                      DeviceCard(device: device, controller: c),
                  ],
                ),
              ],
              if (tab == 1) ..._invitations(),
              if (tab == 2) MobileSettings(controller: c),
              if (tab == 3) ...[
                Card(
                  child: Padding(
                    padding: const EdgeInsets.all(22),
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        const CircleAvatar(
                          radius: 28,
                          child: Icon(Icons.person_outline_rounded, size: 30),
                        ),
                        const SizedBox(height: 16),
                        Text(
                          c.account?['email'] as String? ?? '',
                          style: Theme.of(context).textTheme.titleMedium,
                        ),
                        const SizedBox(height: 8),
                        const Text(
                          'Sanser 2.3.8 • Flutter',
                          style: TextStyle(color: Colors.white54),
                        ),
                        const SizedBox(height: 24),
                        TextButton(
                          onPressed: () => showLicensePage(
                            context: context,
                            applicationName: 'Sanser',
                            applicationVersion: mobileVersion,
                          ),
                          child: const Text('Giấy phép phần mềm'),
                        ),
                        OutlinedButton.icon(
                          onPressed: () => c.logout(),
                          icon: const Icon(Icons.logout_rounded),
                          label: const Text('Đăng xuất'),
                        ),
                      ],
                    ),
                  ),
                ),
                const SizedBox(height: 18),
                const Text(
                  'Khi rời ứng dụng, kết nối từ xa được đóng và phím đang giữ được thả.',
                  style: TextStyle(color: Colors.white54),
                ),
              ],
            ],
          ),
        ),
      );
    },
  );
  List<Widget> _invitations() {
    final cards = <Widget>[];
    for (final group in c.sharing) {
      for (final member
          in (group['members'] as List? ?? []).whereType<Json>()) {
        if (member['viewerDeviceId'] != c.deviceId ||
            member['state'] != 'invited') {
          continue;
        }
        cards.add(
          Card(
            child: Padding(
              padding: const EdgeInsets.all(20),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  const Icon(
                    Icons.screen_share_outlined,
                    color: Color(0xff78dfc0),
                    size: 32,
                  ),
                  const SizedBox(height: 14),
                  Text(
                    group['hostName'] as String? ?? 'Lời mời xem màn hình',
                    style: const TextStyle(
                      fontSize: 19,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                  const SizedBox(height: 8),
                  const Text(
                    'Chỉ xem • Không điều khiển chuột hoặc bàn phím',
                    style: TextStyle(color: Colors.white60),
                  ),
                  const SizedBox(height: 20),
                  Row(
                    children: [
                      Expanded(
                        child: OutlinedButton(
                          onPressed: c.busy
                              ? null
                              : () => c.respond(group, member, false),
                          child: const Text('Từ chối'),
                        ),
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: FilledButton(
                          onPressed: c.busy
                              ? null
                              : () => c.respond(group, member, true),
                          child: const Text('Tham gia'),
                        ),
                      ),
                    ],
                  ),
                ],
              ),
            ),
          ),
        );
        cards.add(const SizedBox(height: 14));
      }
    }
    return cards.isEmpty
        ? const [
            EmptyCard(
              icon: Icons.mark_email_read_outlined,
              title: 'Chưa có lời mời',
              message:
                  'Trên máy tính, chọn Share with multiple computers và mời điện thoại này.',
            ),
          ]
        : cards;
  }
}

class Brand extends StatelessWidget {
  const Brand({super.key});
  @override
  Widget build(BuildContext context) => Row(
    mainAxisSize: MainAxisSize.min,
    children: [
      Image.asset('assets/sanser-mark.png', width: 32, height: 32),
      SizedBox(width: 10),
      Text(
        'Sanser',
        style: TextStyle(fontWeight: FontWeight.w700, letterSpacing: -.7),
      ),
    ],
  );
}

class ErrorNote extends StatelessWidget {
  final String message;
  const ErrorNote(this.message, {super.key});
  @override
  Widget build(BuildContext context) => Container(
    margin: const EdgeInsets.only(bottom: 18),
    padding: const EdgeInsets.all(16),
    decoration: BoxDecoration(
      color: const Color(0xff38252c),
      borderRadius: BorderRadius.circular(16),
    ),
    child: Text(message, style: const TextStyle(color: Color(0xffffb4bc))),
  );
}

class EmptyCard extends StatelessWidget {
  final IconData icon;
  final String title, message;
  const EmptyCard({
    super.key,
    required this.icon,
    required this.title,
    required this.message,
  });
  @override
  Widget build(BuildContext context) => Card(
    child: Padding(
      padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 38),
      child: Column(
        children: [
          Icon(icon, size: 48, color: SanserTokens.accent),
          const SizedBox(height: 18),
          Text(title, style: Theme.of(context).textTheme.titleLarge),
          const SizedBox(height: 10),
          Text(
            message,
            textAlign: TextAlign.center,
            style: const TextStyle(color: Colors.white60, height: 1.5),
          ),
        ],
      ),
    ),
  );
}

class DeviceCard extends StatelessWidget {
  final Json device;
  final MobileController controller;
  const DeviceCard({super.key, required this.device, required this.controller});
  @override
  Widget build(BuildContext context) {
    final online = device['status'] == 'online' || device['online'] == true;
    return Card(
      child: Padding(
        padding: const EdgeInsets.all(20),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Container(
                  padding: const EdgeInsets.all(14),
                  decoration: BoxDecoration(
                    color: SanserTokens.raised,
                    borderRadius: BorderRadius.circular(17),
                  ),
                  child: Icon(
                    (device['platform'] as String? ?? '')
                            .toLowerCase()
                            .contains('mac')
                        ? Icons.laptop_mac_rounded
                        : Icons.desktop_windows_rounded,
                    color: SanserTokens.accent,
                  ),
                ),
                const Spacer(),
                Icon(
                  Icons.circle,
                  size: 8,
                  color: online ? SanserTokens.success : Colors.white38,
                ),
                const SizedBox(width: 7),
                Flexible(
                  child: Text(
                    online ? 'Trực tuyến' : 'Ngoại tuyến',
                    textAlign: TextAlign.end,
                    style: TextStyle(
                      color: online ? SanserTokens.success : Colors.white38,
                    ),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 18),
            Text(
              device['name'] as String? ?? 'Máy tính',
              style: Theme.of(
                context,
              ).textTheme.titleLarge?.copyWith(fontWeight: FontWeight.w600),
            ),
            const SizedBox(height: 5),
            Text(
              device['platform'] as String? ?? '',
              style: const TextStyle(color: Colors.white54),
            ),
            const SizedBox(height: 22),
            Row(
              children: [
                Expanded(
                  child: FilledButton.icon(
                    onPressed:
                        !online ||
                            controller.busy ||
                            controller.capabilities['available'] != true
                        ? null
                        : () => controller.connect(device),
                    icon: const Icon(Icons.arrow_outward_rounded),
                    label: const Text('Kết nối'),
                  ),
                ),
                const SizedBox(width: 10),
                IconButton.filledTonal(
                  tooltip: 'Chỉ xem',
                  onPressed:
                      !online ||
                          controller.busy ||
                          controller.capabilities['available'] != true
                      ? null
                      : () => controller.connect(device, viewOnly: true),
                  icon: const Icon(Icons.visibility_outlined),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

class SignIn extends StatefulWidget {
  final MobileController controller;
  const SignIn({super.key, required this.controller});
  @override
  State<SignIn> createState() => _SignInState();
}

class _SignInState extends State<SignIn> {
  final email = TextEditingController(),
      password = TextEditingController(),
      server = TextEditingController(
        text: const String.fromEnvironment(
          'SANSER_SERVER_URL',
          defaultValue: 'https://sanser.onrender.com',
        ),
      );
  final form = GlobalKey<FormState>();
  bool create = false, visible = false;
  @override
  void dispose() {
    email.dispose();
    password.dispose();
    server.dispose();
    super.dispose();
  }

  void submit() {
    if (form.currentState!.validate()) {
      unawaited(
        widget.controller.login(
          email.text,
          password.text,
          server.text,
          register: create,
        ),
      );
    }
  }

  @override
  Widget build(BuildContext context) => Scaffold(
    body: SafeArea(
      child: Center(
        child: SingleChildScrollView(
          padding: const EdgeInsets.all(26),
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 460),
            child: Form(
              key: form,
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  const Align(alignment: Alignment.centerLeft, child: Brand()),
                  const SizedBox(height: 56),
                  Text(
                    'Máy tính của bạn.\nTrong tầm tay.',
                    style: Theme.of(context).textTheme.headlineLarge?.copyWith(
                      fontWeight: FontWeight.w700,
                      height: 1.17,
                      letterSpacing: -1,
                    ),
                  ),
                  const SizedBox(height: 14),
                  const Text(
                    'Dùng cùng tài khoản Sanser trên điện thoại và máy tính.',
                    style: TextStyle(color: Colors.white60, height: 1.5),
                  ),
                  const SizedBox(height: 30),
                  if (widget.controller.error != null)
                    ErrorNote(widget.controller.error!),
                  TextFormField(
                    controller: email,
                    autofillHints: const [AutofillHints.email],
                    keyboardType: TextInputType.emailAddress,
                    textInputAction: TextInputAction.next,
                    decoration: const InputDecoration(
                      labelText: 'Email',
                      prefixIcon: Icon(Icons.mail_outline_rounded),
                    ),
                    validator: (value) => value != null && value.contains('@')
                        ? null
                        : 'Nhập email của bạn',
                  ),
                  const SizedBox(height: 14),
                  TextFormField(
                    controller: password,
                    obscureText: !visible,
                    autofillHints: [
                      create
                          ? AutofillHints.newPassword
                          : AutofillHints.password,
                    ],
                    onFieldSubmitted: (_) => submit(),
                    decoration: InputDecoration(
                      labelText: 'Mật khẩu',
                      prefixIcon: const Icon(Icons.lock_outline_rounded),
                      suffixIcon: IconButton(
                        tooltip: visible ? 'Ẩn mật khẩu' : 'Hiện mật khẩu',
                        onPressed: () => setState(() => visible = !visible),
                        icon: Icon(
                          visible
                              ? Icons.visibility_off_outlined
                              : Icons.visibility_outlined,
                        ),
                      ),
                    ),
                    validator: (value) => value != null && value.isNotEmpty
                        ? null
                        : 'Nhập mật khẩu',
                  ),
                  const SizedBox(height: 22),
                  FilledButton(
                    onPressed: widget.controller.busy ? null : submit,
                    child: widget.controller.busy
                        ? const SizedBox(
                            height: 22,
                            width: 22,
                            child: CircularProgressIndicator(strokeWidth: 2),
                          )
                        : Text(create ? 'Tạo tài khoản' : 'Đăng nhập'),
                  ),
                  TextButton(
                    onPressed: widget.controller.busy
                        ? null
                        : () => setState(() => create = !create),
                    child: Text(
                      create
                          ? 'Đã có tài khoản? Đăng nhập'
                          : 'Chưa có tài khoản? Đăng ký',
                    ),
                  ),
                  const SizedBox(height: 20),
                  ExpansionTile(
                    tilePadding: EdgeInsets.zero,
                    title: const Text(
                      'Máy chủ riêng',
                      style: TextStyle(fontSize: 13, color: Colors.white54),
                    ),
                    children: [
                      TextFormField(
                        controller: server,
                        keyboardType: TextInputType.url,
                        decoration: const InputDecoration(
                          labelText: 'Địa chỉ HTTPS',
                        ),
                        validator: (value) {
                          try {
                            serverUri(value ?? '');
                            return null;
                          } catch (_) {
                            return 'Nhập địa chỉ HTTPS hợp lệ';
                          }
                        },
                      ),
                      const SizedBox(height: 10),
                    ],
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    ),
  );
}

class RemoteSession extends StatefulWidget {
  final MobileController controller;
  const RemoteSession({super.key, required this.controller});
  @override
  State<RemoteSession> createState() => _RemoteSessionState();
}

class _RemoteSessionState extends State<RemoteSession>
    with WidgetsBindingObserver {
  bool? _landscape;
  late final MobileInput input;
  late final MobileFiles files;
  late final MobileHaptics haptics;
  late final GamepadKeyCapture gamepadInput;
  late TouchMode mode;
  bool gameControls = false;
  bool externalKeyboard = false;
  bool controllerKeys = false,
      controllerAnalog = false,
      _gamepadUpdatePending = false;
  bool soundBusy = false;
  int resetZoom = 0;
  final Set<int> modifiers = {};
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    final revision = c.mediaRevision, sessionId = c.active?['id'];
    input = MobileInput(
      c.engine,
      c.active?['id'] as String? ?? '',
      enabled: c.active?['accessMode'] != 'view-only',
      current: () =>
          mounted &&
          c.mediaRevision == revision &&
          c.active?['id'] == sessionId &&
          c.phase == SessionPhase.streaming,
    );
    mode = c.active?['accessMode'] == 'view-only'
        ? TouchMode.pan
        : c.preferences.pointerMode == 'touchpad'
        ? TouchMode.touchpad
        : TouchMode.direct;
    files = MobileFiles(
      c.engine,
      input.sessionId,
      current: () => mounted && inputReady,
      start: c.active?['accessMode'] != 'view-only',
    );
    files.addListener(_filesChanged);
    haptics = MobileHaptics(
      enabled: () => c.preferences.haptics,
      current: () =>
          mounted && inputReady && (ModalRoute.of(context)?.isCurrent ?? false),
    );
    gamepadInput = GamepadKeyCapture(
      c.engine,
      input,
      attempt: () => c.engineAttempt,
      current: () =>
          mounted &&
          inputReady &&
          (ModalRoute.of(context)?.isCurrent ?? false) &&
          (WidgetsBinding.instance.lifecycleState == null ||
              WidgetsBinding.instance.lifecycleState ==
                  AppLifecycleState.resumed),
      onReleased: () {
        if (mounted) {
          setState(() {
            controllerKeys = false;
            controllerAnalog = false;
          });
        }
      },
    )..addListener(_gamepadChanged);
    c.addListener(_analogAvailabilityChanged);
    _orientation();
  }

  void _analogAvailabilityChanged() {
    if (!mounted ||
        !controllerAnalog ||
        (inputReady && c.telemetry['gamepad'] == true)) {
      return;
    }
    gamepadInput.setCapture(false);
    scheduleMicrotask(() {
      if (mounted) setState(() => controllerAnalog = false);
    });
  }

  void _gamepadChanged() {
    if (!mounted || _gamepadUpdatePending) return;
    _gamepadUpdatePending = true;
    scheduleMicrotask(() {
      _gamepadUpdatePending = false;
      if (mounted) setState(() {});
    });
  }

  void _filesChanged() {
    if (mounted) setState(() {});
  }

  Future<void> _files() async {
    if (!inputReady) return;
    gamepadInput.setCapture(false);
    await input.releaseAll();
    if (!mounted || !inputReady) return;
    setState(() => modifiers.clear());
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      builder: (_) => FileTransfersSheet(files: files),
    );
  }

  @override
  void didUpdateWidget(covariant RemoteSession oldWidget) {
    super.didUpdateWidget(oldWidget);
    _orientation();
  }

  void _orientation() {
    final value = c.preferences.autoLandscape;
    if (value == _landscape) return;
    _landscape = value;
    unawaited(
      SystemChrome.setPreferredOrientations(
        value
            ? [
                DeviceOrientation.landscapeLeft,
                DeviceOrientation.landscapeRight,
              ]
            : [],
      ).catchError((Object _) {}),
    );
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    c.removeListener(_analogAvailabilityChanged);
    haptics.dispose();
    gamepadInput.removeListener(_gamepadChanged);
    gamepadInput.dispose();
    input.dispose();
    files.dispose();
    unawaited(
      SystemChrome.setPreferredOrientations([
        DeviceOrientation.portraitUp,
      ]).catchError((Object _) {}),
    );
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state != AppLifecycleState.resumed && modifiers.isNotEmpty) {
      setState(() => modifiers.clear());
    }
  }

  Future<void> _keyboard() async {
    if (!inputReady) return;
    gamepadInput.setCapture(false);
    await input.releaseAll();
    if (!mounted || !inputReady) return;
    setState(() => modifiers.clear());
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (_) => ListenableBuilder(
        listenable: c,
        builder: (_, _) => RemoteKeyboardSheet(
          input: input,
          current: () => mounted && inputReady,
          unicodeSupported: c.telemetry['unicodeInput'] == true,
        ),
      ),
    );
  }

  Future<void> _clipboard() async {
    if (!inputReady || c.telemetry['clipboard'] != true) return;
    gamepadInput.setCapture(false);
    await input.releaseAll();
    if (!mounted || !inputReady) return;
    setState(() => modifiers.clear());
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (_) => ClipboardSheet(
        engine: c.engine,
        sessionId: input.sessionId,
        current: () => mounted && inputReady,
      ),
    );
  }

  Future<void> _shortcuts() async {
    if (!inputReady) return;
    gamepadInput.setCapture(false);
    FocusManager.instance.primaryFocus?.unfocus();
    await input.releaseAll();
    if (!mounted || !inputReady) return;
    setState(() => modifiers.clear());
    ShortcutHost? host;
    final hostId = c.active?['hostDeviceId'];
    if (hostId is String && hostId.isNotEmpty) {
      for (final device in c.devices) {
        if (device['id'] == hostId) {
          host = shortcutHost(device['platform'] as String?);
          break;
        }
      }
    }
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      builder: (_) => ListenableBuilder(
        listenable: c,
        builder: (_, _) => HostShortcutsSheet(
          input: input,
          current: () => mounted && inputReady,
          detectedHost: host,
        ),
      ),
    );
  }

  MobileController get c => widget.controller;
  Size get videoSize {
    final w = c.telemetry['videoWidth'], h = c.telemetry['videoHeight'];
    return w is num && h is num && w > 0 && h > 0
        ? Size(w.toDouble(), h.toDouble())
        : const Size(1920, 1080);
  }

  Future<void> _key(int code) async {
    if (!inputReady) return;
    try {
      await input.tapKey(code);
    } catch (_) {
      await c.engine.resetInput().catchError((Object _) {});
      if (mounted) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text('Không gửi được phím. Kiểm tra kết nối.'),
          ),
        );
      }
    }
  }

  bool get streamReady =>
      c.phase == SessionPhase.streaming && c.active?['id'] == input.sessionId;
  bool get inputReady =>
      streamReady &&
      c.capabilities['available'] == true &&
      c.active?['accessMode'] != 'view-only';

  Future<void> _toggleSound() async {
    if (soundBusy || !streamReady || c.capabilities['audio'] != true) return;
    setState(() => soundBusy = true);
    try {
      await c.setSound(!c.sound);
    } finally {
      if (mounted) setState(() => soundBusy = false);
    }
  }

  void _changeMode(TouchMode next) {
    if (!inputReady) return;
    gamepadInput.setCapture(false);
    setState(() {
      mode = next;
      controllerKeys = controllerAnalog = false;
      modifiers.clear();
    });
    unawaited(input.releaseAll());
    if (next != TouchMode.pan) {
      unawaited(
        c.setPreferences(c.preferences.copyWith(pointerMode: next.name)),
      );
    }
  }

  Widget _controls(bool viewOnly) => Column(
    mainAxisSize: MainAxisSize.min,
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      if (viewOnly)
        const Padding(
          padding: EdgeInsets.only(bottom: 12),
          child: Text(
            'Phiên chỉ xem · chụm và kéo để phóng to hình.',
            style: TextStyle(color: SanserTokens.muted, fontSize: 12),
          ),
        )
      else ...[
        SegmentedButton<TouchMode>(
          showSelectedIcon: false,
          segments: const [
            ButtonSegment(
              value: TouchMode.direct,
              icon: Icon(Icons.touch_app_outlined),
              tooltip: 'Chạm trực tiếp',
            ),
            ButtonSegment(
              value: TouchMode.touchpad,
              icon: Icon(Icons.mouse_outlined),
              tooltip: 'Touchpad',
            ),
            ButtonSegment(
              value: TouchMode.pan,
              icon: Icon(Icons.zoom_in_map_rounded),
              tooltip: 'Phóng to và di chuyển hình',
            ),
          ],
          selected: {mode},
          onSelectionChanged: inputReady
              ? (selection) => _changeMode(selection.single)
              : null,
        ),
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 10),
          child: Text(
            mode == TouchMode.touchpad
                ? 'Rê 1 ngón · cuộn 2 ngón · giữ để nhấp phải'
                : mode == TouchMode.pan
                ? 'Chụm để zoom · kéo để di chuyển hình'
                : 'Chạm để nhấp · kéo để di chuyển · giữ để nhấp phải',
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: 11, color: SanserTokens.muted),
          ),
        ),
      ],
      SessionActionGrid(
        children: [
          if (c.capabilities['audio'] == true)
            SessionActionButton(
              icon: c.sound
                  ? Icons.volume_up_outlined
                  : Icons.volume_off_outlined,
              label: c.sound ? 'Tắt âm thanh' : 'Bật âm thanh',
              selected: c.sound,
              busy: soundBusy,
              onPressed: streamReady ? _toggleSound : null,
            ),
          if (!viewOnly)
            SessionActionButton(
              icon: Icons.keyboard_outlined,
              label: 'Bàn phím',
              onPressed: inputReady ? _keyboard : null,
            ),
          if (!viewOnly)
            SessionActionButton(
              icon: Icons.keyboard_command_key,
              label: 'Phím tắt máy tính',
              onPressed: inputReady ? _shortcuts : null,
            ),
          if (!viewOnly)
            SessionActionButton(
              icon: Icons.keyboard_alt_outlined,
              label: 'Bàn phím ngoài',
              selected: externalKeyboard,
              onPressed: inputReady
                  ? () => setState(() {
                      externalKeyboard = !externalKeyboard;
                      controllerKeys = false;
                      controllerAnalog = false;
                    })
                  : null,
            ),
          if (!viewOnly && c.capabilities['controllerKeys'] == true)
            SessionActionButton(
              icon: Icons.sports_esports_outlined,
              label: 'Tay cầm → phím',
              selected: controllerKeys,
              onPressed: inputReady
                  ? () => setState(() {
                      gamepadInput.setAnalog(false);
                      controllerKeys = !controllerKeys;
                      controllerAnalog = false;
                      externalKeyboard = false;
                    })
                  : null,
            ),
          if (!viewOnly && c.capabilities['controllerAnalog'] == true)
            SessionActionButton(
              icon: Icons.videogame_asset_outlined,
              label: 'Tay cầm analog',
              selected: controllerAnalog,
              onPressed: inputReady && c.telemetry['gamepad'] == true
                  ? () => setState(() {
                      gamepadInput.setAnalog(true);
                      controllerAnalog = !controllerAnalog;
                      controllerKeys = false;
                      externalKeyboard = false;
                    })
                  : null,
            ),
          if (!viewOnly)
            SessionActionButton(
              icon: Icons.content_paste_outlined,
              label: 'Clipboard',
              onPressed: inputReady && c.telemetry['clipboard'] == true
                  ? _clipboard
                  : null,
            ),
          if (!viewOnly)
            SessionActionButton(
              icon: Icons.folder_open_outlined,
              label: files.invitation ? 'File gửi đến' : 'Truyền file',
              selected: files.invitation,
              onPressed: inputReady && files.available ? _files : null,
            ),
          SessionActionButton(
            icon: Icons.center_focus_strong,
            label: 'Vừa khung',
            onPressed: streamReady ? () => setState(() => resetZoom++) : null,
          ),
          SessionActionButton(
            icon: Icons.settings_outlined,
            label: 'Cài đặt phiên',
            onPressed: () {
              gamepadInput.setCapture(false);
              FocusManager.instance.primaryFocus?.unfocus();
              showMobileSettings(context, c);
            },
          ),
        ],
      ),
      if (externalKeyboard && !viewOnly)
        const Padding(
          padding: EdgeInsets.only(top: 8),
          child: Text(
            'Bấm vào hình để gửi phím USB/Bluetooth. Ctrl+Alt+Esc để nhả.',
            style: TextStyle(fontSize: 11, color: SanserTokens.muted),
          ),
        ),
      if (controllerKeys && !viewOnly)
        Padding(
          padding: const EdgeInsets.only(top: 8),
          child: Text(
            gamepadInput.error ??
                (!gamepadInput.connected
                    ? 'Chưa nhận tay cầm. Kết nối USB/Bluetooth rồi bấm vào hình.'
                    : !gamepadInput.armed
                    ? 'Đã nhận tay cầm. Thả nút và đưa cần về giữa để bắt đầu.'
                    : 'WASD · mũi tên · A: Space, B: Esc, X: E, Y: R · LB/RB: Q/F · LT/RT: Shift/Ctrl · Start/Select: Enter/Tab. Giữ Start+Select để nhả. Không giả lập Xbox.'),
            style: const TextStyle(fontSize: 11, color: SanserTokens.muted),
          ),
        ),
      if (controllerAnalog && !viewOnly)
        Padding(
          padding: const EdgeInsets.only(top: 8),
          child: Text(
            gamepadInput.error ??
                (!gamepadInput.connected
                    ? 'Kết nối tay cầm USB/Bluetooth rồi bấm vào hình.'
                    : !gamepadInput.armed
                    ? 'Thả các nút, cò và đưa cần về giữa để bật analog.'
                    : 'Analog · cần và cò giữ mức lực · Start+Select để nhả. Chưa hỗ trợ rung.'),
            style: const TextStyle(fontSize: 11, color: SanserTokens.muted),
          ),
        ),
      if (!viewOnly &&
          c.capabilities['controllerAnalog'] == true &&
          c.telemetry['gamepad'] != true)
        const Padding(
          padding: EdgeInsets.only(top: 8),
          child: Text(
            'Analog cần host Windows hỗ trợ tay cầm ảo. Bạn vẫn có thể dùng Tay cầm → phím.',
            style: TextStyle(fontSize: 11, color: SanserTokens.muted),
          ),
        ),
      if (c.capabilities['audio'] != true)
        const Padding(
          padding: EdgeInsets.only(top: 8),
          child: Text(
            'Bản engine này chưa hỗ trợ âm thanh.',
            style: TextStyle(fontSize: 11, color: SanserTokens.muted),
          ),
        ),
      const SizedBox(height: 8),
      if (!viewOnly)
        ExpansionTile(
          tilePadding: const EdgeInsets.symmetric(horizontal: 4),
          title: const Text(
            'Phím và điều khiển',
            style: TextStyle(fontSize: 13),
          ),
          leading: const Icon(Icons.tune_rounded, size: 19),
          children: [
            Wrap(
              spacing: 6,
              runSpacing: 6,
              children: [
                for (final item in const [
                  (41, 'Esc'),
                  (43, 'Tab'),
                  (40, 'Enter'),
                  (42, '⌫'),
                ])
                  OutlinedButton(
                    onPressed: inputReady ? () => _key(item.$1) : null,
                    child: Text(item.$2),
                  ),
              ],
            ),
            Wrap(
              spacing: 4,
              runSpacing: 4,
              children: [
                for (final key in const [
                  (224, 'Ctrl'),
                  (225, 'Shift'),
                  (226, 'Alt'),
                  (227, '⌘/Win'),
                ])
                  FilterChip(
                    label: Text(key.$2),
                    selected: modifiers.contains(key.$1),
                    onSelected: !inputReady
                        ? null
                        : (down) {
                            setState(() {
                              if (down) {
                                modifiers.add(key.$1);
                              } else {
                                modifiers.remove(key.$1);
                              }
                            });
                            unawaited(input.hold(key.$1, down, 'modifier'));
                          },
                  ),
              ],
            ),
            TextButton.icon(
              icon: const Icon(Icons.keyboard_hide_outlined, size: 18),
              label: const Text('Thả phím đang giữ'),
              onPressed: !inputReady
                  ? null
                  : () {
                      gamepadInput.setCapture(false);
                      setState(() {
                        modifiers.clear();
                        controllerKeys = controllerAnalog = false;
                      });
                      unawaited(input.releaseAll());
                    },
            ),
            FilterChip(
              label: const Text('Phím game'),
              avatar: const Icon(Icons.sports_esports_outlined, size: 18),
              selected: gameControls,
              onSelected: !inputReady
                  ? null
                  : (value) {
                      gamepadInput.setCapture(false);
                      unawaited(input.releaseAll());
                      setState(() {
                        gameControls = value;
                        controllerKeys = controllerAnalog = false;
                        modifiers.clear();
                      });
                    },
            ),
            if (gameControls)
              const Padding(
                padding: EdgeInsets.symmetric(vertical: 8),
                child: Text(
                  'WASD + phím bàn phím · không phải tay cầm Xbox',
                  style: TextStyle(color: SanserTokens.muted, fontSize: 11),
                ),
              ),
          ],
        ),
      ExpansionTile(
        tilePadding: const EdgeInsets.symmetric(horizontal: 4),
        leading: const Icon(Icons.network_check_outlined, size: 19),
        title: const Text('Thông tin kết nối', style: TextStyle(fontSize: 13)),
        children: [
          Text(
            !streamReady
                ? 'Đang chọn đường truyền…'
                : '${c.networkLabel.isEmpty ? '' : '${c.networkLabel} · '}${c.selectedTransport == 'direct' ? 'Direct UDP' : 'Relay'} • ${c.telemetry['rttMs'] is num ? '${(c.telemetry['rttMs'] as num).toStringAsFixed(1)} ms' : 'Đang đo RTT'}',
            style: const TextStyle(color: SanserTokens.textSoft, fontSize: 12),
          ),
          if (c.telemetry['videoFps'] is num)
            Padding(
              padding: const EdgeInsets.symmetric(vertical: 8),
              child: Text(
                'Nhận ${(c.telemetry['videoFps'] as num).toStringAsFixed(0)} fps${c.telemetry['displayFps'] is num ? ' · Hiển thị ${(c.telemetry['displayFps'] as num).toStringAsFixed(0)} fps' : ''} · NACK ${((c.telemetry['nackRequestsPerSecond'] as num?) ?? 0).toStringAsFixed(0)}/s · Bù âm ${((c.telemetry['audioConcealedPerSecond'] as num?) ?? 0).toStringAsFixed(0)} gói/s',
                style: const TextStyle(fontSize: 11, color: SanserTokens.muted),
              ),
            ),
          const SizedBox(height: 8),
        ],
      ),
      if (c.routeNotice != null)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 8),
          child: Text(
            c.routeNotice!,
            style: const TextStyle(color: SanserTokens.warning, fontSize: 12),
          ),
        ),
      if (c.error != null) ErrorNote(c.error!),
      if (c.active?['sharingId'] != null && c.capabilities['voice'] == true)
        Row(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            IconButton(
              tooltip: c.voiceJoined ? 'Rời voice' : 'Tham gia voice',
              onPressed: !streamReady
                  ? null
                  : () => c.setVoice(joined: !c.voiceJoined, mic: false),
              icon: Icon(c.voiceJoined ? Icons.headset_mic : Icons.headset_off),
            ),
            IconButton(
              tooltip: c.microphone ? 'Tắt mic' : 'Bật mic',
              onPressed: !streamReady || !c.voiceJoined
                  ? null
                  : () => c.setVoice(mic: !c.microphone),
              icon: Icon(c.microphone ? Icons.mic : Icons.mic_off),
            ),
          ],
        ),
    ],
  );
  @override
  Widget build(BuildContext context) {
    final viewOnly = c.active?['accessMode'] == 'view-only';
    return PopScope(
      canPop: false,
      onPopInvokedWithResult: (didPop, result) {
        if (!didPop) unawaited(c.disconnect());
      },
      child: Scaffold(
        backgroundColor: Colors.black,
        body: Stack(
          children: [
            Positioned.fill(
              child: IgnorePointer(
                child: defaultTargetPlatform == TargetPlatform.android
                    ? const AndroidView(viewType: 'com.sanser.mobile/video')
                    : defaultTargetPlatform == TargetPlatform.iOS
                    ? const UiKitView(viewType: 'com.sanser.mobile/video')
                    : const SizedBox.shrink(),
              ),
            ),
            Positioned.fill(
              child: ExternalKeyboardSurface(
                key: ValueKey(
                  'keys:$externalKeyboard-pad:$controllerKeys-analog:$controllerAnalog',
                ),
                input: input,
                enabled:
                    (externalKeyboard ||
                        controllerKeys ||
                        (controllerAnalog && c.telemetry['gamepad'] == true)) &&
                    inputReady,
                forwardKeys: externalKeyboard,
                onFocusChanged: (focused) => gamepadInput.setCapture(
                  (controllerKeys || controllerAnalog) && focused,
                ),
                current: () => mounted && inputReady,
                onReleased: () => setState(() {
                  externalKeyboard = false;
                  modifiers.clear();
                }),
                child: Stack(
                  children: [
                    if (c.phase == SessionPhase.streaming)
                      Positioned.fill(
                        child: MobileTouchSurface(
                          input: input,
                          mode: mode,
                          resetZoom: resetZoom,
                          videoSize: videoSize,
                          onTapFeedback: haptics.tap,
                          onHoldFeedback: haptics.hold,
                        ),
                      ),
                    if (gameControls && !viewOnly && streamReady)
                      Positioned.fill(
                        child: MobileGameControls(
                          input: input,
                          onPressFeedback: haptics.tap,
                        ),
                      ),
                  ],
                ),
              ),
            ),
            if (files.invitation)
              Positioned(
                left: 12,
                right: 12,
                bottom: MediaQuery.paddingOf(context).bottom + 12,
                child: Material(
                  color: Theme.of(context).colorScheme.surfaceContainerHigh,
                  borderRadius: BorderRadius.circular(16),
                  child: ListTile(
                    leading: const Icon(Icons.file_download_outlined),
                    title: Text('File gửi đến: ${files.incoming?['name']}'),
                    subtitle: const Text('Chưa lưu file. Cần bạn chấp nhận.'),
                    trailing: TextButton(
                      onPressed: _files,
                      child: const Text('Xem'),
                    ),
                  ),
                ),
              ),
            if (c.phase != SessionPhase.streaming)
              Center(
                child: Container(
                  padding: const EdgeInsets.all(28),
                  margin: const EdgeInsets.all(24),
                  decoration: BoxDecoration(
                    color: SanserTokens.surface,
                    borderRadius: BorderRadius.circular(22),
                  ),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      const CircularProgressIndicator(),
                      const SizedBox(height: 22),
                      Text(
                        c.phase == SessionPhase.approval
                            ? 'Đang chờ máy tính chấp nhận…'
                            : c.recovering
                            ? c.recoveryStatus
                            : 'Đang mở màn hình từ xa…',
                        textAlign: TextAlign.center,
                      ),
                      if (c.error != null) ...[
                        const SizedBox(height: 16),
                        ErrorNote(c.error!),
                      ],
                      const SizedBox(height: 12),
                      TextButton(
                        onPressed: () => c.disconnect(),
                        child: const Text('Hủy kết nối'),
                      ),
                    ],
                  ),
                ),
              ),
            FloatingSessionControls(
              initiallyCompact: c.preferences.compactControls,
              expandedHeight: viewOnly ? 320 : 400,
              icon: viewOnly
                  ? Icons.visibility_outlined
                  : Icons.touch_app_outlined,
              title: viewOnly ? 'Chỉ xem' : 'Điều khiển',
              statusLabel: c.phase == SessionPhase.streaming
                  ? '${c.selectedTransport == 'direct' ? 'Direct UDP' : 'Relay'}${c.preferences.showLatency && c.telemetry['rttMs'] is num ? ' · ${(c.telemetry['rttMs'] as num).toStringAsFixed(0)} ms' : ''}'
                  : c.recovering
                  ? 'Đang khôi phục kết nối…'
                  : 'Đang kết nối…',
              connected: c.phase == SessionPhase.streaming,
              content: _controls(viewOnly),
              footer: OutlinedButton.icon(
                style: OutlinedButton.styleFrom(
                  foregroundColor: SanserTokens.danger,
                ),
                icon: const Icon(Icons.logout_rounded, size: 19),
                label: Text(streamReady ? 'Ngắt kết nối' : 'Hủy kết nối'),
                onPressed: () => c.disconnect(),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
