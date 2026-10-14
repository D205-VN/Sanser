#import <ScreenCaptureKit/ScreenCaptureKit.h>
#include "capture_startup.h"
#include <stdexcept>

namespace sanser::desktop {
std::string captureFailure(NSError* error, const char* operation) {
  std::string message = std::string(operation) + ": " + error.localizedDescription.UTF8String
      + " (" + error.domain.UTF8String + " " + std::to_string(error.code) + ")";
  if ([error.domain isEqualToString:SCStreamErrorDomain] && error.code == SCStreamErrorUserDeclined)
    message += ". On the host Mac, allow Sanser in System Settings > Privacy & Security > "
               "Screen & System Audio Recording. If already enabled, quit Sanser completely "
               "(Command+Q), reopen /Applications/Sanser.app and reconnect. An updated "
               "ad-hoc-signed build may need permission granted again.";
  return message;
}

void awaitCaptureOperation(const CaptureOperation& start, const char* operation,
                           std::chrono::milliseconds timeout) {
  if (![NSThread isMainThread]) throw std::runtime_error("Capture startup must run on the main thread");
  __block bool completed = false;
  __block NSError* failure = nil;
  const auto deadline = std::chrono::steady_clock::now() + timeout;
  start(^(NSError* error) {
    dispatch_async(dispatch_get_main_queue(), ^{ failure = error; completed = true; });
  });
  while (!completed && std::chrono::steady_clock::now() < deadline)
    [[NSRunLoop currentRunLoop] runUntilDate:[NSDate dateWithTimeIntervalSinceNow:0.005]];
  if (!completed) throw std::runtime_error(std::string(operation) + " timed out. "
      "Check the host Mac for a permission dialog, then quit and reopen Sanser before reconnecting.");
  if (failure) {
    if (@available(macOS 12.3, *)) throw std::runtime_error(captureFailure(failure, operation));
    throw std::runtime_error(std::string(operation) + " failed");
  }
}
}
