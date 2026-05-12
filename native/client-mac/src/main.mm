#import <Cocoa/Cocoa.h>
#import <CoreMedia/CoreMedia.h>
#import <Metal/Metal.h>
#import <MetalKit/MetalKit.h>
#import <VideoToolbox/VideoToolbox.h>

#include <chrono>
#include <algorithm>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <sstream>
#include <stdexcept>
#include <string>

namespace {

CMVideoCodecType fourcc(char a, char b, char c, char d) {
  return (static_cast<CMVideoCodecType>(a) << 24)
       | (static_cast<CMVideoCodecType>(b) << 16)
       | (static_cast<CMVideoCodecType>(c) << 8)
       | static_cast<CMVideoCodecType>(d);
}

std::string jsonEscape(const char* value) {
  std::ostringstream escaped;
  for (const char* cursor = value ? value : ""; *cursor; ++cursor) {
    const char ch = *cursor;
    switch (ch) {
      case '\\': escaped << "\\\\"; break;
      case '"': escaped << "\\\""; break;
      case '\n': escaped << "\\n"; break;
      case '\r': escaped << "\\r"; break;
      case '\t': escaped << "\\t"; break;
      default: escaped << ch; break;
    }
  }
  return escaped.str();
}

std::string nsStringToUtf8(NSString* value) {
  return value ? std::string([value UTF8String] ?: "") : "";
}

std::string boolText(bool value) {
  return value ? "yes" : "no";
}

id<MTLDevice> defaultMetalDevice() {
  id<MTLDevice> device = MTLCreateSystemDefaultDevice();
  if (device) return device;

  NSArray<id<MTLDevice>>* devices = MTLCopyAllDevices();
  if ([devices count] > 0) return [devices objectAtIndex:0];
  return nil;
}

void printMetalDevices() {
  NSArray<id<MTLDevice>>* devices = MTLCopyAllDevices();
  std::cout << "Metal devices found: " << [devices count] << "\n";
  for (id<MTLDevice> device in devices) {
    std::cout << "  - " << nsStringToUtf8([device name]) << "\n";
  }
}

void printHelp() {
  std::cout
    << "sanser-native-client --probe\n"
    << "sanser-native-client --metal-test [--seconds 5]\n"
    << "sanser-native-client --clipboard-read\n"
    << "sanser-native-client --clipboard-write \"text\"\n"
    << "\n"
    << "Phase 5 prototype:\n"
    << "  --probe           Print VideoToolbox hardware decode and Metal availability\n"
    << "  --metal-test      Open a Metal window and log native input events to stdout\n"
    << "  --seconds N       Auto-close Metal test after N seconds; 0 keeps it open\n"
    << "  --clipboard-read  Print macOS pasteboard text\n"
    << "  --clipboard-write Write text into macOS pasteboard\n";
}

void printCodecSupport(const char* label, CMVideoCodecType codec) {
  bool supported = false;
  if (@available(macOS 10.13, *)) {
    supported = VTIsHardwareDecodeSupported(codec);
  }
  std::cout << "VideoToolbox " << label << " hardware decode: " << boolText(supported) << "\n";
}

int runProbe() {
  @autoreleasepool {
    id<MTLDevice> device = defaultMetalDevice();
    std::cout << "Sanser native mac client probe\n";
    std::cout << "Metal device: " << (device ? nsStringToUtf8([device name]) : "unavailable") << "\n";
    printMetalDevices();
    printCodecSupport("H.264", kCMVideoCodecType_H264);
    printCodecSupport("H.265/HEVC", kCMVideoCodecType_HEVC);
    printCodecSupport("AV1", fourcc('a', 'v', '0', '1'));
  }
  return 0;
}

int clipboardRead() {
  @autoreleasepool {
    NSString* value = [[NSPasteboard generalPasteboard] stringForType:NSPasteboardTypeString];
    if (value) std::cout << [value UTF8String] << "\n";
  }
  return 0;
}

int clipboardWrite(const std::string& value) {
  @autoreleasepool {
    NSPasteboard* pasteboard = [NSPasteboard generalPasteboard];
    [pasteboard clearContents];
    NSString* text = [NSString stringWithUTF8String:value.c_str()];
    if (![pasteboard setString:text forType:NSPasteboardTypeString]) {
      throw std::runtime_error("Failed to write text to macOS pasteboard.");
    }
  }
  return 0;
}

double secondsSince(std::chrono::steady_clock::time_point start) {
  const auto now = std::chrono::steady_clock::now();
  return std::chrono::duration<double>(now - start).count();
}

void logPointerEvent(const char* type, NSEvent* event, NSView* view) {
  const NSPoint point = [view convertPoint:[event locationInWindow] fromView:nil];
  const NSRect bounds = [view bounds];
  const double width = std::max<double>(bounds.size.width, 1.0);
  const double height = std::max<double>(bounds.size.height, 1.0);
  const double x = std::clamp(point.x / width, 0.0, 1.0);
  const double y = std::clamp(1.0 - (point.y / height), 0.0, 1.0);
  std::cout << "SNINPUT {\"type\":\"" << type
            << "\",\"x\":" << x
            << ",\"y\":" << y
            << ",\"button\":" << [event buttonNumber]
            << ",\"dx\":" << [event scrollingDeltaX]
            << ",\"dy\":" << [event scrollingDeltaY]
            << "}\n";
}

void logKeyEvent(const char* type, NSEvent* event) {
  const std::string text = jsonEscape([[event charactersIgnoringModifiers] UTF8String]);
  std::cout << "SNINPUT {\"type\":\"" << type
            << "\",\"key\":\"" << text
            << "\",\"keyCode\":" << [event keyCode]
            << ",\"modifiers\":" << static_cast<unsigned long long>([event modifierFlags])
            << "}\n";
}

} // namespace

@interface SanserMetalView : MTKView
@end

@implementation SanserMetalView

- (BOOL)acceptsFirstResponder {
  return YES;
}

- (void)viewDidMoveToWindow {
  [super viewDidMoveToWindow];
  [[self window] makeFirstResponder:self];
  [[self window] setAcceptsMouseMovedEvents:YES];
}

- (void)keyDown:(NSEvent*)event {
  logKeyEvent("key-down", event);
}

- (void)keyUp:(NSEvent*)event {
  logKeyEvent("key-up", event);
}

- (void)flagsChanged:(NSEvent*)event {
  logKeyEvent("modifiers", event);
}

- (void)mouseMoved:(NSEvent*)event {
  logPointerEvent("pointer-move", event, self);
}

- (void)mouseDragged:(NSEvent*)event {
  logPointerEvent("pointer-move", event, self);
}

- (void)rightMouseDragged:(NSEvent*)event {
  logPointerEvent("pointer-move", event, self);
}

- (void)otherMouseDragged:(NSEvent*)event {
  logPointerEvent("pointer-move", event, self);
}

- (void)mouseDown:(NSEvent*)event {
  logPointerEvent("pointer-down", event, self);
}

- (void)mouseUp:(NSEvent*)event {
  logPointerEvent("pointer-up", event, self);
}

- (void)rightMouseDown:(NSEvent*)event {
  logPointerEvent("pointer-down", event, self);
}

- (void)rightMouseUp:(NSEvent*)event {
  logPointerEvent("pointer-up", event, self);
}

- (void)otherMouseDown:(NSEvent*)event {
  logPointerEvent("pointer-down", event, self);
}

- (void)otherMouseUp:(NSEvent*)event {
  logPointerEvent("pointer-up", event, self);
}

- (void)scrollWheel:(NSEvent*)event {
  logPointerEvent("wheel", event, self);
}

@end

@interface SanserMetalRenderer : NSObject <MTKViewDelegate>
- (instancetype)initWithDevice:(id<MTLDevice>)device view:(MTKView*)view;
@end

@implementation SanserMetalRenderer {
  id<MTLCommandQueue> _commandQueue;
  id<MTLRenderPipelineState> _pipeline;
  std::chrono::steady_clock::time_point _startedAt;
}

- (instancetype)initWithDevice:(id<MTLDevice>)device view:(MTKView*)view {
  self = [super init];
  if (!self) return nil;

  _startedAt = std::chrono::steady_clock::now();
  _commandQueue = [device newCommandQueue];

  NSString* shaderSource =
    @"#include <metal_stdlib>\n"
     @"using namespace metal;\n"
     @"struct VertexOut { float4 position [[position]]; float2 uv; };\n"
     @"vertex VertexOut vertex_main(uint vid [[vertex_id]]) {\n"
     @"  float2 pos[6] = { {-1,-1}, {1,-1}, {-1,1}, {-1,1}, {1,-1}, {1,1} };\n"
     @"  float2 uv[6] = { {0,1}, {1,1}, {0,0}, {0,0}, {1,1}, {1,0} };\n"
     @"  VertexOut out;\n"
     @"  out.position = float4(pos[vid], 0, 1);\n"
     @"  out.uv = uv[vid];\n"
     @"  return out;\n"
     @"}\n"
     @"fragment float4 fragment_main(VertexOut in [[stage_in]], constant float& time [[buffer(0)]]) {\n"
     @"  float scan = 0.08 * sin((in.uv.y + time * 0.18) * 90.0);\n"
     @"  float3 color = mix(float3(0.02, 0.03, 0.07), float3(0.12, 0.72, 0.58), in.uv.x);\n"
     @"  color += float3(0.42, 0.25, 0.92) * (0.35 + 0.25 * sin(time + in.uv.y * 7.0));\n"
     @"  return float4(color + scan, 1.0);\n"
     @"}\n";

  NSError* error = nil;
  id<MTLLibrary> library = [device newLibraryWithSource:shaderSource options:nil error:&error];
  if (!library) {
    std::cerr << "Metal shader compile failed: " << nsStringToUtf8([error localizedDescription]) << "\n";
    return nil;
  }

  MTLRenderPipelineDescriptor* descriptor = [[MTLRenderPipelineDescriptor alloc] init];
  descriptor.vertexFunction = [library newFunctionWithName:@"vertex_main"];
  descriptor.fragmentFunction = [library newFunctionWithName:@"fragment_main"];
  descriptor.colorAttachments[0].pixelFormat = view.colorPixelFormat;

  _pipeline = [device newRenderPipelineStateWithDescriptor:descriptor error:&error];
  if (!_pipeline) {
    std::cerr << "Metal pipeline creation failed: " << nsStringToUtf8([error localizedDescription]) << "\n";
    return nil;
  }

  return self;
}

- (void)mtkView:(MTKView*)view drawableSizeWillChange:(CGSize)size {
  (void)view;
  (void)size;
}

- (void)drawInMTKView:(MTKView*)view {
  MTLRenderPassDescriptor* pass = [view currentRenderPassDescriptor];
  id<CAMetalDrawable> drawable = [view currentDrawable];
  if (!pass || !drawable) return;

  id<MTLCommandBuffer> commandBuffer = [_commandQueue commandBuffer];
  id<MTLRenderCommandEncoder> encoder = [commandBuffer renderCommandEncoderWithDescriptor:pass];

  float time = static_cast<float>(secondsSince(_startedAt));
  [encoder setRenderPipelineState:_pipeline];
  [encoder setFragmentBytes:&time length:sizeof(time) atIndex:0];
  [encoder drawPrimitives:MTLPrimitiveTypeTriangle vertexStart:0 vertexCount:6];
  [encoder endEncoding];

  [commandBuffer presentDrawable:drawable];
  [commandBuffer commit];
}

@end

namespace {

int runMetalTest(double seconds) {
  @autoreleasepool {
    id<MTLDevice> device = defaultMetalDevice();
    if (!device) {
      throw std::runtime_error("Metal is not available on this Mac.");
    }

    [NSApplication sharedApplication];
    [NSApp setActivationPolicy:NSApplicationActivationPolicyRegular];

    const NSRect frame = NSMakeRect(0, 0, 960, 540);
    const NSWindowStyleMask style = NSWindowStyleMaskTitled
                                  | NSWindowStyleMaskClosable
                                  | NSWindowStyleMaskMiniaturizable
                                  | NSWindowStyleMaskResizable;
    NSWindow* window = [[NSWindow alloc] initWithContentRect:frame
                                                   styleMask:style
                                                     backing:NSBackingStoreBuffered
                                                       defer:NO];
    [window setTitle:@"Sanser Native Client - Metal Render Test"];

    SanserMetalView* view = [[SanserMetalView alloc] initWithFrame:frame device:device];
    [view setColorPixelFormat:MTLPixelFormatBGRA8Unorm];
    [view setPreferredFramesPerSecond:60];
    [view setPaused:NO];
    [view setEnableSetNeedsDisplay:NO];

    SanserMetalRenderer* renderer = [[SanserMetalRenderer alloc] initWithDevice:device view:view];
    if (!renderer) {
      throw std::runtime_error("Failed to create Metal renderer.");
    }
    [view setDelegate:renderer];
    [window setContentView:view];
    [window center];
    [window makeKeyAndOrderFront:nil];
    [window makeFirstResponder:view];
    [NSApp activateIgnoringOtherApps:YES];

    std::cout << "Metal render test running on " << nsStringToUtf8([device name]) << "\n";
    std::cout << "Move mouse, click, type, or scroll to see SNINPUT events.\n";

    if (seconds > 0) {
      [NSTimer scheduledTimerWithTimeInterval:seconds
                                      repeats:NO
                                        block:^(NSTimer*) {
                                          [NSApp terminate:nil];
                                        }];
    }

    [NSApp run];
  }
  return 0;
}

struct Options {
  bool probe = false;
  bool metalTest = false;
  bool clipboardRead = false;
  bool clipboardWrite = false;
  bool help = false;
  double seconds = 5;
  std::string clipboardText;
};

Options parseOptions(int argc, char** argv) {
  Options options;
  if (argc <= 1) {
    options.probe = true;
    return options;
  }

  for (int i = 1; i < argc; ++i) {
    const std::string arg = argv[i];
    if (arg == "--probe") {
      options.probe = true;
    } else if (arg == "--metal-test") {
      options.metalTest = true;
    } else if (arg == "--clipboard-read") {
      options.clipboardRead = true;
    } else if (arg == "--clipboard-write") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --clipboard-write");
      options.clipboardWrite = true;
      options.clipboardText = argv[++i];
    } else if (arg == "--seconds") {
      if (i + 1 >= argc) throw std::runtime_error("Missing value for --seconds");
      options.seconds = std::atof(argv[++i]);
    } else if (arg == "--help" || arg == "-h") {
      options.help = true;
    } else {
      throw std::runtime_error("Unknown argument: " + arg);
    }
  }
  return options;
}

} // namespace

int main(int argc, char** argv) {
  try {
    const Options options = parseOptions(argc, argv);
    if (options.help) {
      printHelp();
      return 0;
    }
    if (options.probe) return runProbe();
    if (options.clipboardRead) return clipboardRead();
    if (options.clipboardWrite) return clipboardWrite(options.clipboardText);
    if (options.metalTest) return runMetalTest(options.seconds);

    printHelp();
    return 0;
  } catch (const std::exception& error) {
    std::cerr << "sanser-native-client error: " << error.what() << "\n";
    return 1;
  }
}
