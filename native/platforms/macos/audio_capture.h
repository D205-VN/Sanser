#pragma once
#import <CoreMedia/CoreMedia.h>
#include "audio.h"
#include "voice_echo.h"

namespace sanser::desktop {
// Consumed only on the dedicated ScreenCaptureKit audio queue.
void captureSystemAudio(CMSampleBufferRef sample,AudioPacketizer& output,voice::PlaybackTap* reference=nullptr);
}
