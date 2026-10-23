#pragma once
#include <cstdint>
#include <stdexcept>
#include <string>
#include <utility>

namespace sanser::capture {
// HRESULT values are kept platform-neutral so failure/recovery sequences can
// be tested without a physical display. Windows tests check them against SDK.
inline bool recoverable(std::uint32_t result) {
  switch(result) {
    case 0x00000080: // WAIT_ABANDONED
    case 0x80070005: // E_ACCESSDENIED
    case 0x887a0005: // DXGI_ERROR_DEVICE_REMOVED
    case 0x887a0007: // DXGI_ERROR_DEVICE_RESET
    case 0x887a0020: // DXGI_ERROR_DRIVER_INTERNAL_ERROR
    case 0x887a0022: // DXGI_ERROR_NOT_CURRENTLY_AVAILABLE
    case 0x887a0026: // DXGI_ERROR_ACCESS_LOST
    case 0x887a0028: // DXGI_ERROR_SESSION_DISCONNECTED
    case 0x887a002b: // DXGI_ERROR_ACCESS_DENIED (different from E_ACCESSDENIED)
      return true;
    default: return false;
  }
}
class OperationError : public std::runtime_error {
public:
  OperationError(std::uint32_t result,std::string message):std::runtime_error(std::move(message)),result_(result) {}
  std::uint32_t result() const { return result_; }
private:
  std::uint32_t result_;
};
// Cleanup in work must unwind first. Recreate on the existing bounded retry
// path instead of letting a transient GPU/desktop failure terminate streaming.
template<class Work,class Recover> bool guard(Work work,Recover recover) {
  try { return work(); }
  catch(const OperationError& error) {
    if(!recoverable(error.result())) throw;
    recover(error); return false;
  }
}
}
