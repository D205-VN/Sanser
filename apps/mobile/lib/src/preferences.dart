import 'api.dart';

class MobilePreferences {
  final String networkMode;
  final String pointerMode;
  final int videoBitrateMbps;
  final bool autoLandscape,
      soundOnConnect,
      compactControls,
      showLatency,
      haptics;
  const MobilePreferences({
    this.networkMode = 'auto',
    this.pointerMode = 'direct',
    this.videoBitrateMbps = 6,
    this.autoLandscape = true,
    this.soundOnConnect = true,
    this.compactControls = true,
    this.showLatency = true,
    this.haptics = false,
  });
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
  }) => MobilePreferences(
    networkMode: networkMode ?? this.networkMode,
    pointerMode: pointerMode ?? this.pointerMode,
    videoBitrateMbps: videoBitrateMbps ?? this.videoBitrateMbps,
    autoLandscape: autoLandscape ?? this.autoLandscape,
    soundOnConnect: soundOnConnect ?? this.soundOnConnect,
    compactControls: compactControls ?? this.compactControls,
    showLatency: showLatency ?? this.showLatency,
    haptics: haptics ?? this.haptics,
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
  };
}
