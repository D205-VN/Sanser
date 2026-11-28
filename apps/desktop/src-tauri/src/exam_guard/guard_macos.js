// Sanser exam guard for macOS. Run by exam_guard.rs through osascript with
// one JSON argument. Prints one JSON object per line on stdout.
ObjC.import('AppKit');
ObjC.import('CoreGraphics');

function out(value) {
  $.NSFileHandle.fileHandleWithStandardOutput.writeData($(JSON.stringify(value) + '\n').dataUsingEncoding($.NSUTF8StringEncoding));
}
function text(value) { return value && !value.isNil() ? value.js : ''; }
function matchesAny(value, words) {
  var lower = value.toLowerCase();
  for (var i = 0; i < words.length; i += 1) if (lower.indexOf(words[i]) >= 0) return true;
  return false;
}
function frontWindowTitle(pid) {
  // Window names need Screen Recording permission, which exam mode asks for.
  var list = ObjC.castRefToObject($.CGWindowListCopyWindowInfo(1 | 16, 0));
  for (var i = 0; i < list.count; i += 1) {
    var item = list.objectAtIndex(i);
    if (item.objectForKey('kCGWindowOwnerPID').js !== pid || item.objectForKey('kCGWindowLayer').js !== 0) continue;
    return text(item.objectForKey('kCGWindowName'));
  }
  return '';
}

function run(argv) {
  var config = JSON.parse(argv[0]);
  var allowed = config.allowed.map(function (id) { return id.toLowerCase(); });
  var system = config.system.map(function (id) { return id.toLowerCase(); });
  var self = config.selfId.toLowerCase();
  var sites = config.aiTitles.map(function (source) { return new RegExp(source, 'i'); });
  var workspace = $.NSWorkspace.sharedWorkspace;
  var lastFront = null, lastTitle = null, tick = 0;
  out({ t: 'ready' });
  while (true) {
    tick += 1;
    if (config.blockAi && tick % 3 === 0) {
      var apps = workspace.runningApplications;
      for (var i = 0; i < apps.count; i += 1) {
        var app = apps.objectAtIndex(i);
        if (app.activationPolicy !== 0) continue;
        var appId = text(app.bundleIdentifier), appName = text(app.localizedName);
        var lower = appId.toLowerCase();
        if (lower === self || allowed.indexOf(lower) >= 0 || !matchesAny(appId + ' ' + appName, config.aiApps)) continue;
        app.forceTerminate;
        out({ t: 'ai-app', d: appName || appId });
      }
    }
    var front = workspace.frontmostApplication;
    var id = front.isNil() ? '' : text(front.bundleIdentifier);
    var name = front.isNil() ? '' : text(front.localizedName);
    var lowerId = id.toLowerCase();
    if (id !== lastFront) { out({ t: 'front', d: name || id }); lastFront = id; lastTitle = null; }
    var ok = lowerId === '' || lowerId === self || allowed.indexOf(lowerId) >= 0 || system.indexOf(lowerId) >= 0;
    var bringBack = null;
    if (!ok) {
      out({ t: 'left-app', d: name || id });
      front.hide;
      var running = workspace.runningApplications;
      for (var j = 0; j < running.count && !bringBack; j += 1) {
        var candidate = running.objectAtIndex(j);
        if (allowed.indexOf(text(candidate.bundleIdentifier).toLowerCase()) >= 0) bringBack = candidate;
      }
      if (!bringBack) bringBack = $.NSRunningApplication.runningApplicationWithProcessIdentifier(config.selfPid);
    } else if (config.blockAi && lowerId !== self && !front.isNil()) {
      var title = frontWindowTitle(front.processIdentifier);
      if (title && title !== lastTitle) {
        lastTitle = title;
        for (var k = 0; k < sites.length; k += 1) {
          if (sites[k].test(title)) {
            out({ t: 'ai-site', d: (name ? name + ': ' : '') + title });
            lastTitle = null;
            bringBack = $.NSRunningApplication.runningApplicationWithProcessIdentifier(config.selfPid);
            break;
          }
        }
      }
    }
    if (bringBack && !bringBack.isNil()) bringBack.activateWithOptions(3);
    // Spin the run loop so NSWorkspace sees which app is in front now.
    $.NSRunLoop.currentRunLoop.runUntilDate($.NSDate.dateWithTimeIntervalSinceNow(config.intervalMs / 1000));
  }
}
