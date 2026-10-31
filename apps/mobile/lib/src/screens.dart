import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'api.dart';
import 'controller.dart';
import 'keyboard.dart';

class SanserHome extends StatefulWidget {
  final MobileController controller;
  const SanserHome({super.key, required this.controller});
  @override
  State<SanserHome> createState() => _SanserHomeState();
}

class _SanserHomeState extends State<SanserHome> with WidgetsBindingObserver {
  int tab = 0;
  MobileController get c => widget.controller;
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.paused ||
        state == AppLifecycleState.detached) {
      unawaited(c.disconnect());
    }
    if (state == AppLifecycleState.inactive) {
      unawaited(c.engine.resetInput().catchError((Object _) {}));
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => ListenableBuilder(
    listenable: c,
    builder: (context, _) {
      if (c.account == null) return SignIn(controller: c);
      if (c.active != null) return RemoteSession(controller: c);
      return Scaffold(
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
                ['Máy tính của bạn', 'Lời mời chia sẻ', 'Tài khoản'][tab],
                style: Theme.of(context).textTheme.headlineMedium?.copyWith(
                  fontWeight: FontWeight.w700,
                ),
              ),
              const SizedBox(height: 8),
              Text(
                [
                  'Kết nối và mang máy tính theo bên mình.',
                  'Xem màn hình được chia sẻ với bạn.',
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
                for (final device in c.devices) ...[
                  DeviceCard(device: device, controller: c),
                  const SizedBox(height: 14),
                ],
              ],
              if (tab == 1) ..._invitations(),
              if (tab == 2) ...[
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
                          'Sanser 2.3.1 • Flutter',
                          style: TextStyle(color: Colors.white54),
                        ),
                        const SizedBox(height: 24),
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
        bottomNavigationBar: NavigationBar(
          selectedIndex: tab,
          onDestinationSelected: (value) => setState(() => tab = value),
          destinations: const [
            NavigationDestination(
              icon: Icon(Icons.devices_rounded),
              label: 'Máy tính',
            ),
            NavigationDestination(
              icon: Icon(Icons.screen_share_outlined),
              label: 'Chia sẻ',
            ),
            NavigationDestination(
              icon: Icon(Icons.person_outline_rounded),
              label: 'Tài khoản',
            ),
          ],
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
          Icon(icon, size: 48, color: const Color(0xffb9c8ff)),
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
                    color: const Color(0xff283349),
                    borderRadius: BorderRadius.circular(17),
                  ),
                  child: Icon(
                    (device['platform'] as String? ?? '')
                            .toLowerCase()
                            .contains('mac')
                        ? Icons.laptop_mac_rounded
                        : Icons.desktop_windows_rounded,
                    color: const Color(0xffb9c8ff),
                  ),
                ),
                const Spacer(),
                Icon(
                  Icons.circle,
                  size: 8,
                  color: online ? const Color(0xff78dfc0) : Colors.white38,
                ),
                const SizedBox(width: 7),
                Text(
                  online ? 'Trực tuyến' : 'Ngoại tuyến',
                  style: TextStyle(
                    color: online ? const Color(0xff78dfc0) : Colors.white38,
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

class _RemoteSessionState extends State<RemoteSession> {
  Future<void> _keyboard() async {
    final text = TextEditingController();
    await showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (sheet) => Padding(
        padding: EdgeInsets.fromLTRB(
          20,
          24,
          20,
          MediaQuery.viewInsetsOf(sheet).bottom + 24,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const Text(
              'Bàn phím từ xa',
              style: TextStyle(fontSize: 20, fontWeight: FontWeight.w600),
            ),
            const SizedBox(height: 16),
            TextField(
              controller: text,
              autofocus: true,
              maxLength: 256,
              decoration: const InputDecoration(
                labelText: 'Chữ Latin không dấu, số và ký hiệu',
              ),
            ),
            const SizedBox(height: 12),
            FilledButton(
              onPressed: () async {
                try {
                  await sendText(c.engine, text.text);
                  if (sheet.mounted) Navigator.pop(sheet);
                } catch (error) {
                  if (sheet.mounted) {
                    ScaffoldMessenger.of(
                      sheet,
                    ).showSnackBar(SnackBar(content: Text(error.toString())));
                  }
                }
              },
              child: const Text('Gửi'),
            ),
          ],
        ),
      ),
    );
    // The route's reverse animation may still own its editable field briefly.
    await Future<void>.delayed(const Duration(milliseconds: 350));
    text.dispose();
  }

  bool expanded = false;
  MobileController get c => widget.controller;
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
              child: defaultTargetPlatform == TargetPlatform.android
                  ? const AndroidView(viewType: 'com.sanser.mobile/video')
                  : defaultTargetPlatform == TargetPlatform.iOS
                  ? const UiKitView(viewType: 'com.sanser.mobile/video')
                  : const SizedBox.shrink(),
            ),
            if (c.phase != SessionPhase.streaming)
              Center(
                child: Container(
                  padding: const EdgeInsets.all(28),
                  margin: const EdgeInsets.all(24),
                  decoration: BoxDecoration(
                    color: const Color(0xff191f2c),
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
                            : 'Đang mở màn hình từ xa…',
                        textAlign: TextAlign.center,
                      ),
                      if (c.error != null) ...[
                        const SizedBox(height: 16),
                        ErrorNote(c.error!),
                      ],
                    ],
                  ),
                ),
              ),
            SafeArea(
              child: Align(
                alignment: Alignment.topRight,
                child: Container(
                  margin: const EdgeInsets.all(12),
                  padding: const EdgeInsets.all(6),
                  constraints: const BoxConstraints(maxWidth: 360),
                  decoration: BoxDecoration(
                    color: const Color(0xf2191f2c),
                    borderRadius: BorderRadius.circular(22),
                    border: Border.all(color: Colors.white12),
                  ),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          const SizedBox(width: 8),
                          Icon(
                            viewOnly
                                ? Icons.visibility_outlined
                                : Icons.touch_app_outlined,
                            color: const Color(0xff78dfc0),
                          ),
                          const SizedBox(width: 10),
                          Flexible(
                            child: Text(
                              viewOnly ? 'Chỉ xem' : 'Điều khiển',
                              style: const TextStyle(
                                fontWeight: FontWeight.w600,
                              ),
                            ),
                          ),
                          IconButton(
                            tooltip: c.sound ? 'Tắt âm thanh' : 'Bật âm thanh',
                            onPressed: () => c.setSound(!c.sound),
                            icon: Icon(
                              c.sound
                                  ? Icons.volume_up_outlined
                                  : Icons.volume_off_outlined,
                            ),
                          ),
                          IconButton(
                            tooltip: 'Tùy chọn',
                            onPressed: () =>
                                setState(() => expanded = !expanded),
                            icon: Icon(
                              expanded
                                  ? Icons.expand_less_rounded
                                  : Icons.expand_more_rounded,
                            ),
                          ),
                          IconButton(
                            tooltip: 'Ngắt kết nối',
                            onPressed: () => c.disconnect(),
                            icon: const Icon(
                              Icons.stop_circle_outlined,
                              color: Color(0xffffa4ad),
                            ),
                          ),
                        ],
                      ),
                      if (expanded) ...[
                        const Divider(height: 1),
                        Padding(
                          padding: const EdgeInsets.all(10),
                          child: Column(
                            children: [
                              Text(
                                'Relay • ${c.telemetry['rttMs'] is num ? '${(c.telemetry['rttMs'] as num).toStringAsFixed(1)} ms' : 'Đang đo RTT'}',
                                style: const TextStyle(
                                  color: Colors.white54,
                                  fontSize: 12,
                                ),
                              ),
                              if (!viewOnly)
                                TextButton.icon(
                                  onPressed: _keyboard,
                                  icon: const Icon(Icons.keyboard_outlined),
                                  label: const Text('Bàn phím'),
                                ),
                              if (!viewOnly)
                                Wrap(
                                  spacing: 6,
                                  children: [
                                    for (final item in const [
                                      (41, 'Esc'),
                                      (43, 'Tab'),
                                      (40, 'Enter'),
                                      (42, '⌫'),
                                    ])
                                      OutlinedButton(
                                        onPressed: () async {
                                          await c.engine.key(item.$1, true);
                                          await c.engine.key(item.$1, false);
                                        },
                                        child: Text(item.$2),
                                      ),
                                  ],
                                ),
                              if (c.active?['sharingId'] != null &&
                                  c.capabilities['voice'] == true)
                                Row(
                                  mainAxisAlignment: MainAxisAlignment.center,
                                  children: [
                                    IconButton(
                                      tooltip: c.voiceJoined
                                          ? 'Rời voice'
                                          : 'Tham gia voice',
                                      onPressed: () => c.setVoice(
                                        joined: !c.voiceJoined,
                                        mic: false,
                                      ),
                                      icon: Icon(
                                        c.voiceJoined
                                            ? Icons.headset_mic
                                            : Icons.headset_off,
                                      ),
                                    ),
                                    IconButton(
                                      tooltip: c.microphone
                                          ? 'Tắt mic'
                                          : 'Bật mic',
                                      onPressed: !c.voiceJoined
                                          ? null
                                          : () =>
                                                c.setVoice(mic: !c.microphone),
                                      icon: Icon(
                                        c.microphone
                                            ? Icons.mic
                                            : Icons.mic_off,
                                      ),
                                    ),
                                    IconButton(
                                      tooltip: 'Tắt/bật voice',
                                      onPressed: !c.voiceJoined
                                          ? null
                                          : () => c.setVoice(deaf: !c.deafened),
                                      icon: Icon(
                                        c.deafened
                                            ? Icons.hearing_disabled
                                            : Icons.hearing,
                                      ),
                                    ),
                                  ],
                                ),
                            ],
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
