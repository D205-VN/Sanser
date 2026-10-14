#pragma once
#import <Foundation/Foundation.h>
#include <chrono>
#include <functional>
#include <string>

namespace sanser::desktop {
using CaptureCompletion = void (^)(NSError*);
using CaptureOperation = std::function<void(CaptureCompletion)>;
// Main-thread run-loop wait; callbacks may arrive on any queue, even after timeout.
void awaitCaptureOperation(const CaptureOperation& start, const char* operation,
                           std::chrono::milliseconds timeout = std::chrono::seconds(10));
std::string captureFailure(NSError* error, const char* operation) API_AVAILABLE(macos(12.3));
}
