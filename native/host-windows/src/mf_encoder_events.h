#pragma once

#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <wrl/client.h>
#include <cstdint>
#include <stdexcept>
#include "desktop_duplication.h"

// Async MFT calls consume one matching event each. Never use synchronous
// NEED_MORE_INPUT polling against an asynchronous hardware encoder.
class MfEncoderEvents {
public:
  void initialize(IMFMediaEventGenerator* generator) { generator_ = generator; }
  bool asynchronous() const { return generator_.Get() != nullptr; }
  void collect() {
    if (!generator_) return;
    for (unsigned count = 0; count < 256; ++count) {
      Microsoft::WRL::ComPtr<IMFMediaEvent> event;
      const HRESULT hr = generator_->GetEvent(MF_EVENT_FLAG_NO_WAIT, event.GetAddressOf());
      if (hr == MF_E_NO_EVENTS_AVAILABLE) return;
      check(hr, "Get encoder event");
      HRESULT status = S_OK;
      check(event->GetStatus(&status), "Get encoder event status");
      check(status, "Asynchronous encoder");
      MediaEventType type = MEUnknown;
      check(event->GetType(&type), "Get encoder event type");
      if (type == METransformNeedInput && !draining_) ++inputs_;
      else if (type == METransformHaveOutput) ++outputs_;
      else if (type == METransformDrainComplete) drainComplete_ = true;
      if (inputs_ > 256 || outputs_ > 256) throw std::runtime_error("Encoder event queue overflow");
    }
  }
  bool inputReady() const { return !asynchronous() || inputs_ != 0; }
  void consumeInput() {
    if (asynchronous()) {
      if (!inputs_) throw std::runtime_error("Encoder input without METransformNeedInput");
      --inputs_;
    }
  }
  bool takeOutput() {
    if (!asynchronous()) return true;
    if (!outputs_) return false;
    --outputs_;
    return true;
  }
  void beginDrain() { inputs_ = 0; draining_ = true; drainComplete_ = false; }
  bool drained() const { return drainComplete_ && outputs_ == 0; }
private:
  static void check(HRESULT hr, const char* operation) {
    if (FAILED(hr)) throw std::runtime_error(std::string(operation) + ": " + hresultMessage(hr));
  }
  Microsoft::WRL::ComPtr<IMFMediaEventGenerator> generator_;
  std::uint32_t inputs_ = 0, outputs_ = 0;
  bool draining_ = false, drainComplete_ = false;
};
