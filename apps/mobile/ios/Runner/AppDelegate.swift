import Flutter
import UIKit
import Darwin

@main
@objc class AppDelegate: FlutterAppDelegate, FlutterImplicitEngineDelegate {
  override func application(
    _ application: UIApplication,
    didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]?
  ) -> Bool {
    #if DEBUG || SANSER_NETWORK_DIAGNOSTICS
    if ProcessInfo.processInfo.arguments.contains("--sanser-probe-direct") {
      DispatchQueue.global(qos:.utility).async { Self.probeDirect() }
    }
    #endif
    return super.application(application, didFinishLaunchingWithOptions: launchOptions)
  }

  func didInitializeImplicitFlutterEngine(_ engineBridge: FlutterImplicitEngineBridge) {
    GeneratedPluginRegistrant.register(with: engineBridge.pluginRegistry)
    if let registrar = engineBridge.pluginRegistry.registrar(forPlugin: "SanserMobileEngine") {
      MobileEngine.register(with: registrar)
    }
  }
  #if DEBUG || SANSER_NETWORK_DIAGNOSTICS
  // Explicit developer diagnostic: no login, peer traffic, endpoints or keys
  // are stored. Uses the same native boundary as the production connection.
  private static func probeDirect(){
    let attempt=UUID().uuidString.lowercased()
    var report:[String:Any]=["runId":UUID().uuidString]
    var list:UnsafeMutablePointer<ifaddrs>?
    if getifaddrs(&list)==0 {
      defer{freeifaddrs(list)}
      var item=list,ipv4=0,ipv6=0
      while let entry=item {
        if let address=entry.pointee.ifa_addr {
          let family=Int32(address.pointee.sa_family)
          if entry.pointee.ifa_flags & UInt32(IFF_LOOPBACK)==0 {
            if family==AF_INET{ipv4+=1}
            if family==AF_INET6{ipv6+=1}
          }
        }
        item=entry.pointee.ifa_next
      }
      report["ipv4Interfaces"]=ipv4;report["ipv6Interfaces"]=ipv6
    }
    do{
      let gathered=try MobileEngine.p2p(["operation":"gather","attemptId":attempt,"sessionId":UUID().uuidString.lowercased(),"generation":Int(Date().timeIntervalSince1970),"stunServers":["stun:stun.l.google.com:19302"]])
      report["ok"]=true;report["candidateCount"]=(gathered["candidates"] as? [Any])?.count ?? 0;report["stunSucceeded"]=gathered["stunSucceeded"]
    }catch{report["ok"]=false;report["error"]=error.localizedDescription}
    _=try? MobileEngine.p2p(["operation":"close","attemptId":attempt])
    if let documents=FileManager.default.urls(for:.documentDirectory,in:.userDomainMask).first,
       let data=try? JSONSerialization.data(withJSONObject:report,options:[.sortedKeys]) {
      try? data.write(to:documents.appendingPathComponent("sanser-direct-probe.json"),options:.atomic)
    }
  }
  #endif

}
