#pragma once
#import <MetalKit/MetalKit.h>
#import <CoreVideo/CoreVideo.h>

// Native, on-demand presentation. Retains the newest frame and limits GPU work
// to one submission; neither the WebView nor NSImage participates in video.
@interface SanserMetalView : MTKView <MTKViewDelegate>
@property(nonatomic,readonly) NSSize imageSize;
- (void)presentPixels:(CVPixelBufferRef)pixels;
@end
