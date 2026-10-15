#pragma once
#import <CoreMedia/CoreMedia.h>
#include "audio.h"

namespace sanser::desktop {
// Consumed only on the dedicated ScreenCaptureKit audio queue.
void captureSystemAudio(CMSampleBufferRef sample,AudioPacketizer& output);
}
