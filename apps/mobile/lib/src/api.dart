import 'dart:async';
import 'dart:convert';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:http/http.dart' as http;
import 'package:uuid/uuid.dart';

typedef Json = Map<String, dynamic>;
const mobileVersion = '2.3.1';

abstract interface class SecretStore {
  Future<String?> read(String key);
  Future<void> write(String key, String value);
  Future<void> delete(String key);
}

class DeviceSecrets implements SecretStore {
  final _storage = const FlutterSecureStorage(
    aOptions: AndroidOptions(encryptedSharedPreferences: true),
  );
  @override
  Future<String?> read(String key) => _storage.read(key: key);
  @override
  Future<void> write(String key, String value) =>
      _storage.write(key: key, value: value);
  @override
  Future<void> delete(String key) => _storage.delete(key: key);
}

class ApiFailure implements Exception {
  final int status;
  final String message;
  const ApiFailure(this.status, this.message);
  @override
  String toString() => message;
}

Uri serverUri(String raw) {
  final uri = Uri.tryParse(raw.trim());
  if (uri == null ||
      !uri.hasAuthority ||
      uri.userInfo.isNotEmpty ||
      uri.hasQuery ||
      uri.hasFragment ||
      uri.scheme != 'https') {
    throw const ApiFailure(
      0,
      'Địa chỉ máy chủ phải dùng HTTPS, không chứa tài khoản hoặc tham số.',
    );
  }
  return uri.replace(path: uri.path.replaceFirst(RegExp(r'/+$'), ''));
}

class SanserApi {
  final SecretStore secrets;
  final http.Client client;
  late Uri base;
  String? _access;
  String? _refresh;
  Future<bool>? _renewing;
  int _generation = 0;
  SanserApi(this.secrets, {http.Client? client})
    : client = client ?? http.Client();
  void useServer(String url) {
    base = serverUri(url);
    _access = null;
    _refresh = null;
    _renewing = null;
    _generation++;
  }

  String _key(String name) => 'mobile.${base.toString()}.$name';
  Future<void> loadServer(String fallback) async {
    final selected = await secrets.read('mobile.selected-server');
    useServer(selected ?? fallback);
  }

  Future<String> identity(String name) async {
    final key = _key(name);
    final existing = await secrets.read(key);
    if (existing != null && RegExp(r'^[0-9a-f-]{36}$').hasMatch(existing)) {
      return existing;
    }
    final id = const Uuid().v4();
    await secrets.write(key, id);
    return id;
  }

  Future<void> _tokens(Json result, int generation) async {
    if (generation != _generation) return;
    final access = result['accessToken'];
    final refresh = result['refreshToken'];
    if (access is! String || refresh is! String) {
      throw const ApiFailure(0, 'Máy chủ trả dữ liệu đăng nhập không hợp lệ.');
    }
    final key = _key('refresh');
    await secrets.write(key, refresh);
    if (generation != _generation) return;
    _access = access;
    _refresh = refresh;
  }

  Future<bool> restore() async {
    final generation = _generation;
    final refresh = await secrets.read(_key('refresh'));
    if (generation != _generation) return false;
    _refresh = refresh;
    return _refresh != null && await renew();
  }

  Future<Json> login(
    String email,
    String password, {
    required String deviceName,
    required String platform,
    bool register = false,
  }) async {
    final generation = ++_generation;
    final result = await request(
      register ? '/api/v2/auth/register' : '/api/v2/auth/login',
      method: 'POST',
      auth: false,
      body: {
        'email': email.trim(),
        'password': password,
        'deviceName': deviceName,
        'platform': platform,
        'installationId': await identity('installation'),
        if (register) 'displayName': email.split('@').first,
      },
    );
    await _tokens(result, generation);
    if (generation == _generation) {
      await secrets.write('mobile.selected-server', base.toString());
    }
    return result;
  }

  Future<bool> renew() =>
      _renewing ??= _renew().whenComplete(() => _renewing = null);
  Future<bool> _renew() async {
    final token = _refresh;
    if (token == null) return false;
    final generation = _generation;
    final refreshKey = _key('refresh');
    try {
      final result = await request(
        '/api/v2/auth/refresh',
        method: 'POST',
        auth: false,
        body: {'refreshToken': token},
      );
      if (generation != _generation) return false;
      await _tokens(result, generation);
      return true;
    } on ApiFailure catch (error) {
      if (generation == _generation &&
          (error.status == 401 || error.status == 403)) {
        _access = null;
        _refresh = null;
        await secrets.delete(refreshKey);
      }
      return false;
    }
  }

  Future<String> accessToken() async {
    if (_access == null && !await renew()) {
      throw const ApiFailure(401, 'Vui lòng đăng nhập lại.');
    }
    return _access!;
  }

  Future<void> logout() async {
    ++_generation;
    final token = _refresh;
    final refreshKey = _key('refresh');
    try {
      if (_access != null) {
        await request(
          '/api/v2/auth/logout',
          method: 'POST',
          body: {'refreshToken': token},
          retry: false,
        );
      }
    } finally {
      _access = null;
      _refresh = null;
      await secrets.delete(refreshKey);
    }
  }

  Future<Json> request(
    String path, {
    String method = 'GET',
    Json? body,
    bool auth = true,
    bool retry = true,
  }) async {
    if (!path.startsWith('/api/v2/') || path.startsWith('//')) {
      throw ArgumentError('API path');
    }
    final generation = _generation;
    final uri = Uri.parse('${base.toString()}$path');
    final request = http.Request(method, uri)
      ..headers['Accept'] = 'application/json';
    if (auth) {
      request.headers['Authorization'] = 'Bearer ${await accessToken()}';
    }
    if (generation != _generation) {
      throw const ApiFailure(0, 'Máy chủ đã thay đổi. Vui lòng thử lại.');
    }
    if (body != null) {
      request.headers['Content-Type'] = 'application/json';
      request.body = jsonEncode(body);
    }
    http.StreamedResponse response;
    try {
      response = await client
          .send(request)
          .timeout(const Duration(seconds: 15));
    } on TimeoutException {
      throw const ApiFailure(0, 'Máy chủ phản hồi quá lâu. Vui lòng thử lại.');
    } on http.ClientException {
      throw const ApiFailure(
        0,
        'Không thể liên hệ máy chủ. Kiểm tra kết nối mạng.',
      );
    }
    final bytes = <int>[];
    await for (final chunk in response.stream.timeout(
      const Duration(seconds: 15),
    )) {
      bytes.addAll(chunk);
      if (bytes.length > 1024 * 1024) {
        throw const ApiFailure(0, 'Phản hồi máy chủ vượt giới hạn.');
      }
    }
    if (response.statusCode == 401 && auth && retry && await renew()) {
      return requestAgain(path, method, body);
    }
    dynamic decoded;
    try {
      decoded = bytes.isEmpty
          ? <String, dynamic>{}
          : jsonDecode(utf8.decode(bytes));
    } on FormatException {
      throw const ApiFailure(0, 'Phản hồi máy chủ không phải JSON hợp lệ.');
    }
    if (response.statusCode >= 400) {
      final raw = decoded is Map ? decoded['error'] : null;
      final message = raw is Map
          ? raw['message']
          : decoded is Map
          ? decoded['message']
          : null;
      throw ApiFailure(
        response.statusCode,
        message is String
            ? message
            : 'Yêu cầu không thành công (${response.statusCode}).',
      );
    }
    if (decoded is List) return {'items': decoded};
    if (decoded is! Map<String, dynamic>) {
      throw const ApiFailure(0, 'Cấu trúc phản hồi không hợp lệ.');
    }
    return decoded;
  }

  Future<Json> requestAgain(String path, String method, Json? body) =>
      request(path, method: method, body: body, retry: false);
  Future<List<Json>> list(String path) async {
    final result = await request(path);
    final items = result['items'];
    if (items is! List) {
      throw const ApiFailure(0, 'Danh sách từ máy chủ không hợp lệ.');
    }
    return items.whereType<Json>().toList();
  }

  void close() => client.close();
}
