#pragma once
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#include <shlobj.h>
#include <filesystem>
#include <stdexcept>
#include <string>
#include <vector>
#include <utility>

namespace sanser::uac {
struct Handle {
  HANDLE value=nullptr;
  Handle()=default;
  explicit Handle(HANDLE h):value(h) {}
  ~Handle() { if(*this) CloseHandle(value); }
  Handle(const Handle&)=delete;
  Handle& operator=(const Handle&)=delete;
  Handle(Handle&& h) noexcept:value(std::exchange(h.value,nullptr)) {}
  Handle& operator=(Handle&& h) noexcept { if(this!=&h) { Handle old(std::exchange(value,std::exchange(h.value,nullptr))); } return *this; }
  explicit operator bool() const { return value && value!=INVALID_HANDLE_VALUE; }
};
inline void require(bool ok,const char* name) { if(!ok) throw std::runtime_error(std::string(name)+" (Win32 "+std::to_string(GetLastError())+")"); }
inline std::filesystem::path imagePath() {
  std::vector<wchar_t> path(32768); const DWORD n=GetModuleFileNameW(nullptr,path.data(),static_cast<DWORD>(path.size()));
  require(n>0 && n<path.size(),"Executable path"); return std::wstring(path.data(),n);
}
inline std::filesystem::path installDirectory() {
  PWSTR path=nullptr; const auto hr=SHGetKnownFolderPath(FOLDERID_ProgramFiles,KF_FLAG_DEFAULT,nullptr,&path);
  if(FAILED(hr)) throw std::runtime_error("Program Files location unavailable");
  std::filesystem::path result=std::filesystem::path(path)/L"SanserUac"; CoTaskMemFree(path); return result;
}
inline bool localSystem(HANDLE token) {
  DWORD n=0; GetTokenInformation(token,TokenUser,nullptr,0,&n); std::vector<unsigned char> data(n);
  if(!n || !GetTokenInformation(token,TokenUser,data.data(),n,&n)) return false;
  return IsWellKnownSid(reinterpret_cast<TOKEN_USER*>(data.data())->User.Sid,WinLocalSystemSid)!=FALSE;
}
// Synchronous handles are confined to the SYSTEM worker's inherited pipes.
inline bool transfer(HANDLE pipe,void* data,DWORD size,bool write) {
  auto* cursor=static_cast<unsigned char*>(data);
  while(size) { DWORD n=0; if(!(write?WriteFile(pipe,cursor,size,&n,nullptr):ReadFile(pipe,cursor,size,&n,nullptr)) || !n) return false; cursor+=n; size-=n; }
  return true;
}
// No UI thread or capture thread can wait indefinitely for a dead service.
inline bool timedTransfer(HANDLE pipe,void* data,DWORD size,bool write,DWORD timeout) {
  const auto deadline=GetTickCount64()+timeout; auto* cursor=static_cast<unsigned char*>(data);
  Handle event(CreateEventW(nullptr,TRUE,FALSE,nullptr)); if(!event) return false;
  while(size) {
    OVERLAPPED io{}; io.hEvent=event.value; ResetEvent(event.value); DWORD n=0;
    const BOOL ok=write?WriteFile(pipe,cursor,size,&n,&io):ReadFile(pipe,cursor,size,&n,&io);
    if(!ok) {
      if(GetLastError()!=ERROR_IO_PENDING) return false;
      const auto now=GetTickCount64();
      if(now>=deadline || WaitForSingleObject(event.value,static_cast<DWORD>(deadline-now))!=WAIT_OBJECT_0) {
        CancelIoEx(pipe,&io); WaitForSingleObject(event.value,INFINITE); return false;
      }
      if(!GetOverlappedResult(pipe,&io,&n,FALSE)) return false;
    }
    if(!n) return false; cursor+=n; size-=n;
  }
  return true;
}
} // namespace sanser::uac
