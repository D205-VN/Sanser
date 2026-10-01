#include "gpu_video_processor.h"
#include <d3d11.h>
#include <d3d10.h>
#include <d3dcompiler.h>
#include <mfapi.h>
#include <mferror.h>
#include <algorithm>
#include <cstring>
#include <stdexcept>

using Microsoft::WRL::ComPtr;
namespace {
void check(HRESULT hr, const char* name) {
  if (FAILED(hr)) throw std::runtime_error(std::string(name) + ": " + hresultMessage(hr));
}
bool cursorPixel(int x, int y) {
  return (y >= 0 && y <= 16 && x >= 0 && x <= std::min(8, y / 2 + 1))
      || (y >= 11 && y <= 22 && x >= 3 && x <= 5);
}
struct ContextLock {
  ComPtr<ID3D10Multithread> lock;
  explicit ContextLock(ID3D11DeviceContext* context) {
    check(context->QueryInterface(IID_PPV_ARGS(&lock)), "GPU context lock");
    lock->Enter();
  }
  ~ContextLock() { lock->Leave(); }
};
}
struct GpuVideoProcessor::Impl {
  ComPtr<ID3D11Device> device;
  ComPtr<ID3D11DeviceContext> context;
  ComPtr<ID3D11VideoDevice> video;
  ComPtr<ID3D11VideoContext> videoContext;
  ComPtr<IMFDXGIDeviceManager> manager;
  ComPtr<IMFVideoSampleAllocatorEx> allocator;
  ComPtr<ID3D11VideoProcessorEnumerator> enumerator;
  ComPtr<ID3D11VideoProcessor> processor;
  ComPtr<ID3D11Texture2D> composition;
  ComPtr<ID3D11RenderTargetView> compositionView;
  ComPtr<ID3D11VertexShader> vertex;
  ComPtr<ID3D11PixelShader> pixel;
  ComPtr<ID3D11Buffer> cursorPosition;
  ComPtr<ID3D11RasterizerState> rasterizer;
  ComPtr<ID3D11BlendState> blend;
  std::uint32_t width, height, sourceWidth = 0, sourceHeight = 0;

  void configureSource(const FrameBgra& frame) {
    if (sourceWidth == frame.width && sourceHeight == frame.height) return;
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC description{};
    description.InputFrameFormat = D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE;
    description.InputWidth = frame.width; description.InputHeight = frame.height;
    description.OutputWidth = width; description.OutputHeight = height;
    description.InputFrameRate = {60, 1}; description.OutputFrameRate = {60, 1};
    description.Usage = D3D11_VIDEO_USAGE_OPTIMAL_SPEED;
    check(video->CreateVideoProcessorEnumerator(&description, enumerator.ReleaseAndGetAddressOf()), "GPU video enumerator");
    UINT flags = 0;
    check(enumerator->CheckVideoProcessorFormat(DXGI_FORMAT_B8G8R8A8_UNORM, &flags), "GPU BGRA capability");
    if (!(flags & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_INPUT)) throw std::runtime_error("GPU does not support BGRA input");
    check(enumerator->CheckVideoProcessorFormat(DXGI_FORMAT_NV12, &flags), "GPU NV12 capability");
    if (!(flags & D3D11_VIDEO_PROCESSOR_FORMAT_SUPPORT_OUTPUT)) throw std::runtime_error("GPU does not support NV12 output");
    check(video->CreateVideoProcessor(enumerator.Get(), 0, processor.ReleaseAndGetAddressOf()), "GPU video processor");
    D3D11_TEXTURE2D_DESC texture{};
    texture.Width = frame.width; texture.Height = frame.height;
    texture.MipLevels = 1; texture.ArraySize = 1;
    texture.Format = DXGI_FORMAT_B8G8R8A8_UNORM; texture.SampleDesc.Count = 1;
    texture.Usage = D3D11_USAGE_DEFAULT;
    texture.BindFlags = D3D11_BIND_RENDER_TARGET | D3D11_BIND_SHADER_RESOURCE;
    check(device->CreateTexture2D(&texture, nullptr, composition.ReleaseAndGetAddressOf()), "GPU composition texture");
    check(device->CreateRenderTargetView(composition.Get(), nullptr, compositionView.ReleaseAndGetAddressOf()), "GPU composition view");
    videoContext->VideoProcessorSetStreamFrameFormat(processor.Get(), 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
    videoContext->VideoProcessorSetStreamAutoProcessingMode(processor.Get(), 0, FALSE);
    RECT source{0, 0, static_cast<LONG>(frame.width), static_cast<LONG>(frame.height)};
    RECT destination{0, 0, static_cast<LONG>(width), static_cast<LONG>(height)};
    videoContext->VideoProcessorSetStreamSourceRect(processor.Get(), 0, TRUE, &source);
    videoContext->VideoProcessorSetStreamDestRect(processor.Get(), 0, TRUE, &destination);
    videoContext->VideoProcessorSetOutputTargetRect(processor.Get(), TRUE, &destination);
    D3D11_VIDEO_PROCESSOR_COLOR_SPACE rgb{};
    rgb.Usage = 1; rgb.RGB_Range = 0;
    rgb.Nominal_Range = D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_0_255;
    D3D11_VIDEO_PROCESSOR_COLOR_SPACE yuv{};
    yuv.Usage = 1; yuv.YCbCr_Matrix = 1; // BT.709, limited range.
    yuv.Nominal_Range = D3D11_VIDEO_PROCESSOR_NOMINAL_RANGE_16_235;
    videoContext->VideoProcessorSetStreamColorSpace(processor.Get(), 0, &rgb);
    videoContext->VideoProcessorSetOutputColorSpace(processor.Get(), &yuv);
    sourceWidth = frame.width; sourceHeight = frame.height;
  }

  void drawCursor(const FrameBgra& frame) {
    if (!frame.cursorVisible) return;
    const LONG x = frame.cursorX, y = frame.cursorY;
    RECT scissor{std::max<LONG>(0, x - 1), std::max<LONG>(0, y - 1),
      std::min<LONG>(sourceWidth, x + 10), std::min<LONG>(sourceHeight, y + 24)};
    if (scissor.left >= scissor.right || scissor.top >= scissor.bottom) return;
    const int position[4]{x, y, 0, 0};
    context->UpdateSubresource(cursorPosition.Get(), 0, nullptr, position, 0, 0);
    D3D11_VIEWPORT viewport{0, 0, static_cast<float>(sourceWidth), static_cast<float>(sourceHeight), 0, 1};
    context->RSSetViewports(1, &viewport); context->RSSetScissorRects(1, &scissor);
    context->RSSetState(rasterizer.Get());
    ID3D11RenderTargetView* target = compositionView.Get();
    context->OMSetRenderTargets(1, &target, nullptr);
    context->OMSetBlendState(blend.Get(), nullptr, 0xffffffff);
    context->IASetInputLayout(nullptr);
    context->IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
    context->VSSetShader(vertex.Get(), nullptr, 0);
    context->PSSetShader(pixel.Get(), nullptr, 0);
    ID3D11Buffer* constants = cursorPosition.Get();
    context->PSSetConstantBuffers(0, 1, &constants);
    context->Draw(3, 0);
    context->OMSetRenderTargets(0, nullptr, nullptr);
  }
};

GpuVideoProcessor::GpuVideoProcessor(ID3D11Device* device, std::uint32_t width, std::uint32_t height)
  : impl_(std::make_unique<Impl>()) {
  impl_->device = device; impl_->width = width; impl_->height = height;
  device->GetImmediateContext(&impl_->context);
  check(device->QueryInterface(IID_PPV_ARGS(&impl_->video)), "D3D11 video device");
  check(impl_->context.As(&impl_->videoContext), "D3D11 video context");
  UINT token = 0;
  check(MFCreateDXGIDeviceManager(&token, &impl_->manager), "MF DXGI device manager");
  check(impl_->manager->ResetDevice(device, token), "MF bind capture device");
  check(MFCreateVideoSampleAllocatorEx(IID_PPV_ARGS(&impl_->allocator)), "MF GPU sample allocator");
  check(impl_->allocator->SetDirectXManager(impl_->manager.Get()), "MF GPU allocator device");
  ComPtr<IMFAttributes> attributes;
  check(MFCreateAttributes(&attributes, 1), "MF GPU allocator attributes");
  check(attributes->SetUINT32(MF_SA_D3D11_BINDFLAGS, D3D11_BIND_RENDER_TARGET), "MF GPU allocator binding");
  ComPtr<IMFMediaType> type;
  check(MFCreateMediaType(&type), "MF GPU allocator type");
  check(type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video), "MF GPU allocator video");
  check(type->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_NV12), "MF GPU allocator NV12");
  check(type->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive), "MF GPU allocator progressive");
  check(MFSetAttributeSize(type.Get(), MF_MT_FRAME_SIZE, width, height), "MF GPU allocator size");
  check(impl_->allocator->InitializeSampleAllocatorEx(2, 4, attributes.Get(), type.Get()), "MF bounded GPU sample pool");
  // Draw the existing Sanser cursor in a tiny scissored GPU region. No desktop
  // readback is required, including cursor-only refreshes on an idle desktop.
  static constexpr char shader[] = R"(
    cbuffer Cursor : register(b0) { int2 origin; int2 padding; };
    float4 vs(uint id : SV_VertexID) : SV_Position {
      float2 p = float2((id << 1) & 2, id & 2);
      return float4(p * float2(2, -2) + float2(-1, 1), 0, 1);
    }
    bool arrow(int2 p) {
      return (p.y >= 0 && p.y <= 16 && p.x >= 0 && p.x <= min(8, p.y / 2 + 1))
          || (p.y >= 11 && p.y <= 22 && p.x >= 3 && p.x <= 5);
    }
    float4 ps(float4 position : SV_Position) : SV_Target {
      int2 p = int2(position.xy) - origin;
      if (arrow(p)) return float4(1, 1, 1, 1);
      [unroll] for (int y = -1; y <= 1; ++y)
        [unroll] for (int x = -1; x <= 1; ++x)
          if (arrow(p + int2(x, y))) return float4(0, 0, 0, 1);
      return float4(0, 0, 0, 0);
    }
  )";
  ComPtr<ID3DBlob> code;
  check(D3DCompile(shader, sizeof(shader) - 1, nullptr, nullptr, nullptr, "vs", "vs_4_0", 0, 0, &code, nullptr), "Compile GPU cursor vertex");
  check(device->CreateVertexShader(code->GetBufferPointer(), code->GetBufferSize(), nullptr, &impl_->vertex), "GPU cursor vertex");
  code.Reset();
  check(D3DCompile(shader, sizeof(shader) - 1, nullptr, nullptr, nullptr, "ps", "ps_4_0", 0, 0, &code, nullptr), "Compile GPU cursor pixel");
  check(device->CreatePixelShader(code->GetBufferPointer(), code->GetBufferSize(), nullptr, &impl_->pixel), "GPU cursor pixel");
  D3D11_BUFFER_DESC buffer{};
  buffer.ByteWidth = 16; buffer.Usage = D3D11_USAGE_DEFAULT; buffer.BindFlags = D3D11_BIND_CONSTANT_BUFFER;
  check(device->CreateBuffer(&buffer, nullptr, &impl_->cursorPosition), "GPU cursor constants");
  D3D11_RASTERIZER_DESC raster{};
  raster.FillMode = D3D11_FILL_SOLID; raster.CullMode = D3D11_CULL_NONE; raster.ScissorEnable = TRUE; raster.DepthClipEnable = TRUE;
  check(device->CreateRasterizerState(&raster, &impl_->rasterizer), "GPU cursor rasterizer");
  D3D11_BLEND_DESC blend{};
  auto& target = blend.RenderTarget[0];
  target.BlendEnable = TRUE; target.SrcBlend = D3D11_BLEND_SRC_ALPHA;
  target.DestBlend = D3D11_BLEND_INV_SRC_ALPHA; target.BlendOp = D3D11_BLEND_OP_ADD;
  target.SrcBlendAlpha = D3D11_BLEND_ONE; target.DestBlendAlpha = D3D11_BLEND_ZERO;
  target.BlendOpAlpha = D3D11_BLEND_OP_ADD; target.RenderTargetWriteMask = D3D11_COLOR_WRITE_ENABLE_ALL;
  check(device->CreateBlendState(&blend, &impl_->blend), "GPU cursor blend");
}
GpuVideoProcessor::~GpuVideoProcessor() = default;
IMFDXGIDeviceManager* GpuVideoProcessor::manager() const { return impl_->manager.Get(); }

ComPtr<IMFSample> GpuVideoProcessor::convert(const FrameBgra& frame) {
  if (!frame.texture) throw std::runtime_error("GPU capture unavailable for this display orientation");
  ComPtr<ID3D11Device> sourceDevice;
  frame.texture->GetDevice(&sourceDevice);
  if (sourceDevice.Get() != impl_->device.Get()) throw std::runtime_error("Capture GPU changed; rebuild encoder");
  ComPtr<IMFSample> sample;
  const HRESULT allocation = impl_->allocator->AllocateSample(&sample);
  if (allocation == MF_E_SAMPLEALLOCATOR_EMPTY) return {};
  check(allocation, "MF allocate GPU sample");
  ComPtr<IMFMediaBuffer> buffer;
  check(sample->GetBufferByIndex(0, &buffer), "MF GPU sample buffer");
  ComPtr<IMFDXGIBuffer> dxgi;
  check(buffer.As(&dxgi), "MF GPU sample surface");
  ComPtr<ID3D11Texture2D> output;
  check(dxgi->GetResource(IID_PPV_ARGS(&output)), "MF allocated NV12 texture");
  UINT subresource = 0;
  check(dxgi->GetSubresourceIndex(&subresource), "MF allocated NV12 subresource");
  ContextLock lock(impl_->context.Get());
  impl_->configureSource(frame);
  impl_->context->CopyResource(impl_->composition.Get(), frame.texture.get());
  impl_->drawCursor(frame);
  D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC inputDescription{};
  inputDescription.ViewDimension = D3D11_VPIV_DIMENSION_TEXTURE2D;
  ComPtr<ID3D11VideoProcessorInputView> inputView;
  check(impl_->video->CreateVideoProcessorInputView(impl_->composition.Get(), impl_->enumerator.Get(), &inputDescription, &inputView), "GPU input view");
  D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC outputDescription{};
  D3D11_TEXTURE2D_DESC description{};
  output->GetDesc(&description);
  if (description.ArraySize > 1) {
    outputDescription.ViewDimension = D3D11_VPOV_DIMENSION_TEXTURE2DARRAY;
    outputDescription.Texture2DArray.MipSlice = subresource % description.MipLevels;
    outputDescription.Texture2DArray.FirstArraySlice = subresource / description.MipLevels;
    outputDescription.Texture2DArray.ArraySize = 1;
  } else {
    outputDescription.ViewDimension = D3D11_VPOV_DIMENSION_TEXTURE2D;
    outputDescription.Texture2D.MipSlice = subresource;
  }
  ComPtr<ID3D11VideoProcessorOutputView> outputView;
  check(impl_->video->CreateVideoProcessorOutputView(output.Get(), impl_->enumerator.Get(), &outputDescription, &outputView), "GPU output view");
  D3D11_VIDEO_PROCESSOR_STREAM stream{};
  stream.Enable = TRUE; stream.pInputSurface = inputView.Get();
  check(impl_->videoContext->VideoProcessorBlt(impl_->processor.Get(), outputView.Get(), 0, 1, &stream), "GPU BGRA to NV12");
  impl_->context->Flush(); // Submit GPU work; do not wait for CPU readback.
  ComPtr<IMF2DBuffer> planar;
  check(buffer.As(&planar), "MF GPU planar buffer");
  DWORD length = 0;
  check(planar->GetContiguousLength(&length), "MF GPU input size");
  check(buffer->SetCurrentLength(length), "MF GPU input length");
  return sample;
}

FrameBgra readbackGpuFrame(const FrameBgra& frame) {
  if (!frame.texture) return frame;
  ComPtr<ID3D11Device> device;
  ComPtr<ID3D11DeviceContext> context;
  frame.texture->GetDevice(&device); device->GetImmediateContext(&context);
  D3D11_TEXTURE2D_DESC description{};
  frame.texture->GetDesc(&description);
  description.Usage = D3D11_USAGE_STAGING; description.BindFlags = 0;
  description.CPUAccessFlags = D3D11_CPU_ACCESS_READ; description.MiscFlags = 0;
  ComPtr<ID3D11Texture2D> staging;
  check(device->CreateTexture2D(&description, nullptr, &staging), "Fallback staging texture");
  context->CopyResource(staging.Get(), frame.texture.get());
  FrameBgra result;
  result.width = frame.width; result.height = frame.height; result.stride = frame.width * 4;
  result.pixels.resize(static_cast<size_t>(result.stride) * result.height);
  D3D11_MAPPED_SUBRESOURCE mapped{};
  check(context->Map(staging.Get(), 0, D3D11_MAP_READ, 0, &mapped), "Fallback readback");
  for (std::uint32_t y = 0; y < result.height; ++y)
    std::memcpy(result.pixels.data() + static_cast<size_t>(y) * result.stride,
      static_cast<const std::uint8_t*>(mapped.pData) + static_cast<size_t>(y) * mapped.RowPitch, result.stride);
  context->Unmap(staging.Get(), 0);
  if (frame.cursorVisible) {
    for (int y = -1; y <= 23; ++y) for (int x = -1; x <= 9; ++x) {
      const int px = frame.cursorX + x, py = frame.cursorY + y;
      if (px < 0 || py < 0 || px >= static_cast<int>(frame.width) || py >= static_cast<int>(frame.height)) continue;
      bool outline = false;
      for (int dy = -1; dy <= 1; ++dy) for (int dx = -1; dx <= 1; ++dx) outline |= cursorPixel(x + dx, y + dy);
      if (!outline) continue;
      auto* pixel = result.pixels.data() + static_cast<size_t>(py) * result.stride + px * 4;
      pixel[0] = pixel[1] = pixel[2] = cursorPixel(x, y) ? 255 : 0; pixel[3] = 255;
    }
  }
  return result;
}
