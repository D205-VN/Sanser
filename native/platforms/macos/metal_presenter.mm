#import "metal_presenter.h"
#import <CoreImage/CoreImage.h>
#include <algorithm>
#include <cmath>
#include <stdexcept>
#include <vector>

namespace {
void renderFrame(CIContext* context, CVPixelBufferRef pixels, id<MTLTexture> texture,
                 id<MTLCommandBuffer> command, CGColorSpaceRef colorSpace) {
  const CGRect bounds=CGRectMake(0,0,texture.width,texture.height);
  CIImage* canvas=[[CIImage imageWithColor:CIColor.blackColor] imageByCroppingToRect:bounds];
  if(pixels) {
    CIImage* image=[CIImage imageWithCVPixelBuffer:pixels];
    const double scale=std::min(bounds.size.width/image.extent.size.width,
                                bounds.size.height/image.extent.size.height);
    const double x=(bounds.size.width-image.extent.size.width*scale)/2;
    const double y=(bounds.size.height-image.extent.size.height*scale)/2;
    image=[image imageByApplyingTransform:CGAffineTransformMake(scale,0,0,scale,x,y)];
    canvas=[image imageByCompositingOverImage:canvas];
  }
  // Core Image uses a bottom-left origin; Metal drawables present row zero at
  // the top. Flip the complete canvas so image and absolute pointer agree.
  canvas=[canvas imageByApplyingTransform:CGAffineTransformMake(1,0,0,-1,0,bounds.size.height)];
  [context render:canvas toMTLTexture:texture commandBuffer:command bounds:bounds colorSpace:colorSpace];
}
}

@implementation SanserMetalView {
  id<MTLCommandQueue> _commands;
  CIContext* _context;
  CVPixelBufferRef _pixels;
  CGColorSpaceRef _colorSpace;
  dispatch_semaphore_t _inFlight;
  BOOL _pendingRedraw;
}
- (instancetype)initWithFrame:(NSRect)frame {
  id<MTLDevice> device=MTLCreateSystemDefaultDevice();
  if(!device) throw std::runtime_error("A Metal-capable GPU is required for the remote window");
  self=[super initWithFrame:frame device:device];
  if(self) {
    _commands=[device newCommandQueue];
    _context=[CIContext contextWithMTLDevice:device options:@{kCIContextCacheIntermediates:@NO}];
    if(!_commands || !_context) throw std::runtime_error("Unable to initialize Metal presentation");
    _colorSpace=CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    _inFlight=dispatch_semaphore_create(2);
    self.colorPixelFormat=MTLPixelFormatBGRA8Unorm;
    self.colorspace=_colorSpace;
    self.framebufferOnly=NO;
    self.paused=YES;
    self.enableSetNeedsDisplay=YES;
    self.delegate=self;
  }
  return self;
}
- (void)dealloc {
  if(_pixels) CVPixelBufferRelease(_pixels);
  if(_colorSpace) CGColorSpaceRelease(_colorSpace);
}
- (NSSize)imageSize {
  return _pixels ? NSMakeSize(CVPixelBufferGetWidth(_pixels),CVPixelBufferGetHeight(_pixels)) : NSZeroSize;
}
- (void)presentPixels:(CVPixelBufferRef)pixels {
  if(!pixels) return;
  CVPixelBufferRetain(pixels);
  if(_pixels) CVPixelBufferRelease(_pixels);
  _pixels=pixels;
  _pendingRedraw=YES;
  [self setNeedsDisplay:YES];
}
- (void)mtkView:(MTKView*)view drawableSizeWillChange:(CGSize)size {
  (void)view; (void)size;
  _pendingRedraw=YES;
  [self setNeedsDisplay:YES];
}
- (void)drawInMTKView:(MTKView*)view {
  (void)view;
  if(dispatch_semaphore_wait(_inFlight,DISPATCH_TIME_NOW)!=0) { _pendingRedraw=YES; return; }
  id<CAMetalDrawable> drawable=self.currentDrawable;
  id<MTLCommandBuffer> command=[_commands commandBuffer];
  if(!drawable || !command) { dispatch_semaphore_signal(_inFlight); return; }
  CVPixelBufferRef pixels=_pixels ? CVPixelBufferRetain(_pixels) : nullptr;
  renderFrame(_context,pixels,drawable.texture,command,_colorSpace);
  _pendingRedraw=NO;
  dispatch_semaphore_t slots=_inFlight;
  __weak SanserMetalView* weakSelf=self;
  [command addCompletedHandler:^(id<MTLCommandBuffer>) {
    if(pixels) CVPixelBufferRelease(pixels);
    dispatch_semaphore_signal(slots);
    dispatch_async(dispatch_get_main_queue(), ^{
      SanserMetalView* view=weakSelf;
      if(view && view->_pendingRedraw && view.window.visible) [view setNeedsDisplay:YES];
    });
  }];
  [command presentDrawable:drawable];
  [command commit];
}
@end

#ifdef SANSER_CODEC_TEST
namespace sanser::desktop {
void testMacMetalPresenter() {
  @autoreleasepool {
    id<MTLDevice> device=MTLCreateSystemDefaultDevice();
    if(!device) throw std::runtime_error("Metal presentation test requires a GPU");
    id<MTLCommandQueue> queue=[device newCommandQueue];
    CIContext* context=[CIContext contextWithMTLDevice:device];
    MTLTextureDescriptor* descriptor=[MTLTextureDescriptor texture2DDescriptorWithPixelFormat:MTLPixelFormatBGRA8Unorm width:64 height:64 mipmapped:NO];
    descriptor.usage=MTLTextureUsageShaderRead|MTLTextureUsageShaderWrite|MTLTextureUsageRenderTarget;
    descriptor.storageMode=MTLStorageModeShared;
    id<MTLTexture> texture=[device newTextureWithDescriptor:descriptor];
    if(!texture) throw std::runtime_error("Unable to create Metal test texture");
    CVPixelBufferRef pixels=nullptr;
    const auto status=CVPixelBufferCreate(nullptr,64,32,kCVPixelFormatType_32BGRA,
      (__bridge CFDictionaryRef)@{(__bridge NSString*)kCVPixelBufferIOSurfacePropertiesKey:@{}},&pixels);
    if(status!=kCVReturnSuccess) throw std::runtime_error("Unable to create presenter test image");
    CVPixelBufferLockBaseAddress(pixels,0);
    auto* data=static_cast<unsigned char*>(CVPixelBufferGetBaseAddress(pixels));
    const auto stride=CVPixelBufferGetBytesPerRow(pixels);
    for(unsigned y=0;y<32;++y) for(unsigned x=0;x<64;++x) {
      auto* pixel=data+y*stride+x*4;
      pixel[0]=y<16 ? 30 : 200; pixel[1]=100; pixel[2]=y<16 ? 200 : 30; pixel[3]=255;
    }
    CVPixelBufferUnlockBaseAddress(pixels,0);
    auto colorSpace=CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
    id<MTLCommandBuffer> command=[queue commandBuffer];
    renderFrame(context,pixels,texture,command,colorSpace);
    [command commit]; [command waitUntilCompleted];
    CVPixelBufferRelease(pixels); CGColorSpaceRelease(colorSpace);
    if(command.status!=MTLCommandBufferStatusCompleted) throw std::runtime_error("Metal presentation failed");
    std::vector<unsigned char> result(64*64*4);
    [texture getBytes:result.data() bytesPerRow:64*4 fromRegion:MTLRegionMake2D(0,0,64,64) mipmapLevel:0];
    for(unsigned y : {20U,44U}) {
      const auto* pixel=result.data()+(y*64+32)*4;
      const int blue=y<32 ? 30 : 200, red=y<32 ? 200 : 30;
      if(std::abs(static_cast<int>(pixel[0])-blue)>5 || std::abs(static_cast<int>(pixel[1])-100)>5 || std::abs(static_cast<int>(pixel[2])-red)>5)
        throw std::runtime_error("Metal presenter changed pixel colors or orientation");
    }
    for(unsigned y : {0U,63U}) {
      const auto* bar=result.data()+(y*64+32)*4;
      if(bar[0]!=0 || bar[1]!=0 || bar[2]!=0) throw std::runtime_error("Metal presenter did not preserve aspect ratio");
    }
  }
}
}
#endif
