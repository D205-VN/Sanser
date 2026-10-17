#import <ScreenCaptureKit/ScreenCaptureKit.h>
#include "capture_startup.h"
#include <iostream>
#include <stdexcept>

using namespace sanser::desktop;
using namespace std::chrono_literals;
namespace {
void require(bool value, const char* message) { if (!value) throw std::runtime_error(message); }
CaptureOperation after(std::chrono::milliseconds delay, NSError* error = nil) {
  return [=](CaptureCompletion complete) {
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, delay.count() * NSEC_PER_MSEC),
                   dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{ complete(error); });
  };
}
std::string failure(const CaptureOperation& operation, std::chrono::milliseconds timeout = 1s) {
  try { awaitCaptureOperation(operation, "Test capture", timeout); }
  catch (const std::runtime_error& error) { return error.what(); }
  throw std::runtime_error("Capture failure was ignored");
}
}
int main() {
  @autoreleasepool {
    if (@available(macOS 12.3, *)) {
      try {
        // Success from the capture API is accepted without a CoreGraphics gate
        // or a second permission request. Each phase owns a fresh timeout.
        // Leave scheduling headroom on shared CI runners. This tests callback
        // completion, not whether dispatch_after wakes within a 20 ms margin.
        awaitCaptureOperation(after(30ms), "Enumerate displays", 1s);
        awaitCaptureOperation(after(80ms), "Start stream", 1s);
        NSError* denied = [NSError errorWithDomain:SCStreamErrorDomain code:SCStreamErrorUserDeclined userInfo:nil];
        auto message = failure(after(0ms, denied));
        require(message.find("-3801") != std::string::npos && message.find("Command+Q") != std::string::npos
            && message.find("/Applications/Sanser.app") != std::string::npos, "Permission recovery instructions were lost");
        NSError* invalid = [NSError errorWithDomain:SCStreamErrorDomain code:SCStreamErrorInvalidParameter userInfo:nil];
        message = failure(after(0ms, invalid));
        require(message.find("-3812") != std::string::npos && message.find("allow Sanser") == std::string::npos,
                "Non-permission failures were incorrectly reported as denied access");
        // A late API callback must remain safe after the waiting stack unwinds.
        // Retain the callback explicitly so it is guaranteed to arrive after
        // timeout, even if the CI process is descheduled for an arbitrary time.
        CaptureCompletion late = nil;
        message = failure([&](CaptureCompletion complete) { late = [complete copy]; }, 5ms);
        require(message.find("timed out") != std::string::npos, "Missing callback did not time out");
        require(late != nil, "Late capture callback was not retained");
        late(nil);
        late = nil;
        awaitCaptureOperation(after(60ms), "After late completion", 1s);
        std::cout << "Capture startup: asynchronous success, separate deadlines, denial, actual error, late callback verified\n";
        return 0;
      } catch (const std::exception& error) { std::cerr << error.what() << '\n'; return 1; }
    }
    std::cout << "ScreenCaptureKit requires macOS 12.3\n";
    return 0;
  }
}
