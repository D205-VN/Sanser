import 'api.dart';

class MobilePreferences {
  final String networkMode;
  final int videoBitrateMbps;
  final bool autoLandscape, soundOnConnect, compactControls, showLatency;
  const MobilePreferences({
    this.networkMode = 'auto',
    this.videoBitrateMbps = 6,
    this.autoLandscape = true,
    this.soundOnConnect = true,
    this.compactControls = true,
    this.showLatency = true,
  });
  factory MobilePreferences.fromJson(Json json) => MobilePreferences(
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
  );
  MobilePreferences copyWith({
    String? networkMode,
    int? videoBitrateMbps,
    bool? autoLandscape,
    bool? soundOnConnect,
    bool? compactControls,
    bool? showLatency,
  }) => MobilePreferences(
    networkMode: networkMode ?? this.networkMode,
    videoBitrateMbps: videoBitrateMbps ?? this.videoBitrateMbps,
    autoLandscape: autoLandscape ?? this.autoLandscape,
    soundOnConnect: soundOnConnect ?? this.soundOnConnect,
    compactControls: compactControls ?? this.compactControls,
    showLatency: showLatency ?? this.showLatency,
  );
  Json toJson() => {
    'networkMode': networkMode,
    'videoBitrateMbps': videoBitrateMbps,
    'autoLandscape': autoLandscape,
    'soundOnConnect': soundOnConnect,
    'compactControls': compactControls,
    'showLatency': showLatency,
  };
}
