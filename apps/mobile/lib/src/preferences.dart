import 'api.dart';

class MobilePreferences {
  final String networkMode;
  final String pointerMode;
  final int videoBitrateMbps;

  /// Minutes a session stays connected while Sanser is in the background.
  /// 0 disconnects as soon as the app leaves the screen.
  final int backgroundMinutes;
  final bool autoLandscape,
      soundOnConnect,
      compactControls,
      showLatency,
      haptics,
      pictureInPicture,
      notifications,
      /// Holds one or two frames so the picture moves evenly on uneven
      /// Wi-Fi, at the cost of that much extra delay.
      smoothVideo;
  const MobilePreferences({
    this.networkMode = 'auto',
    this.pointerMode = 'direct',
    this.videoBitrateMbps = 6,
    this.backgroundMinutes = 15,
    this.autoLandscape = true,
    this.soundOnConnect = true,
    this.compactControls = true,
    this.showLatency = true,
    this.haptics = false,
    this.pictureInPicture = true,
    this.notifications = true,
    this.smoothVideo = false,
  });
  static const backgroundChoices = [0, 5, 15, 60];
  factory MobilePreferences.fromJson(Json json) => MobilePreferences(
    pointerMode: ['direct', 'touchpad'].contains(json['pointerMode'])
        ? json['pointerMode']
        : 'direct',
    videoBitrateMbps: [2, 4, 6, 8, 12].contains(json['videoBitrateMbps'])
        ? json['videoBitrateMbps']
        : 6,
    networkMode: ['auto', 'direct', 'relay'].contains(json['networkMode'])
        ? json['networkMode']
        : 'auto',
    autoLandscape: json['autoLandscape'] is bool ? json['autoLandscape'] : true,
    soundOnConnect: json['soundOnConnect'] is bool
        ? json['soundOnConnect']
        : true,
    compactControls: json['compactControls'] is bool
        ? json['compactControls']
        : true,
    showLatency: json['showLatency'] is bool ? json['showLatency'] : true,
    haptics: json['haptics'] is bool ? json['haptics'] : false,
    backgroundMinutes: backgroundChoices.contains(json['backgroundMinutes'])
        ? json['backgroundMinutes']
        : 15,
    pictureInPicture: json['pictureInPicture'] is bool
        ? json['pictureInPicture']
        : true,
    notifications: json['notifications'] is bool ? json['notifications'] : true,
    smoothVideo: json['smoothVideo'] is bool ? json['smoothVideo'] : false,
  );
  MobilePreferences copyWith({
    String? networkMode,
    String? pointerMode,
    int? videoBitrateMbps,
    bool? autoLandscape,
    bool? soundOnConnect,
    bool? compactControls,
    bool? showLatency,
    bool? haptics,
    int? backgroundMinutes,
    bool? pictureInPicture,
    bool? notifications,
    bool? smoothVideo,
  }) => MobilePreferences(
    networkMode: networkMode ?? this.networkMode,
    pointerMode: pointerMode ?? this.pointerMode,
    videoBitrateMbps: videoBitrateMbps ?? this.videoBitrateMbps,
    autoLandscape: autoLandscape ?? this.autoLandscape,
    soundOnConnect: soundOnConnect ?? this.soundOnConnect,
    compactControls: compactControls ?? this.compactControls,
    showLatency: showLatency ?? this.showLatency,
    haptics: haptics ?? this.haptics,
    backgroundMinutes: backgroundMinutes ?? this.backgroundMinutes,
    pictureInPicture: pictureInPicture ?? this.pictureInPicture,
    notifications: notifications ?? this.notifications,
    smoothVideo: smoothVideo ?? this.smoothVideo,
  );
  Json toJson() => {
    'networkMode': networkMode,
    'pointerMode': pointerMode,
    'videoBitrateMbps': videoBitrateMbps,
    'autoLandscape': autoLandscape,
    'soundOnConnect': soundOnConnect,
    'compactControls': compactControls,
    'showLatency': showLatency,
    'haptics': haptics,
    'backgroundMinutes': backgroundMinutes,
    'pictureInPicture': pictureInPicture,
    'notifications': notifications,
    'smoothVideo': smoothVideo,
  };
}
