#pragma once
#include <cstdint>
#include <limits>

namespace sanser::uac {
inline constexpr wchar_t serviceName[]=L"SanserUac";
inline constexpr wchar_t pipeName[]=L"\\\\.\\pipe\\Sanser.Uac.v1";
inline constexpr std::uint32_t magic=0x31435553; // SUC1, local IPC only.
inline constexpr std::uint32_t maxPixelsBytes=64*1024*1024;
enum class Operation : std::uint32_t { Capture=1, Input=2 };
enum class Status : std::uint32_t { Unavailable=0, Frame=1, Idle=2, Applied=3,
  ConsoleUnavailable=4, DesktopUnavailable=5, InputUnavailable=6 };
struct Request {
  std::uint32_t signature=magic;
  Operation operation=Operation::Capture;
  std::uint32_t adapter=0,output=0;
  // Win32 INPUT encoded without pointers, handles, padding or arbitrary blobs.
  std::uint32_t type=0,flags=0,data=0;
  std::int32_t x=0,y=0;
};
struct Response {
  std::uint32_t signature=magic;
  Status status=Status::Unavailable;
  std::uint32_t width=0,height=0,stride=0,bytes=0;
  // Frame: screen origin. Explicit failure statuses: left holds Win32 error;
  // no frame bytes or input content are returned on failure.
  std::int32_t left=0,top=0;
};
static_assert(sizeof(Request)==36 && sizeof(Response)==32);
inline bool valid(const Request& r) {
  if(r.signature!=magic) return false;
  if(r.operation==Operation::Capture)
    return r.adapter<8 && r.output<16 && !r.type && !r.flags && !r.data && !r.x && !r.y;
  if(r.operation!=Operation::Input || r.adapter || r.output) return false;
  if(r.type==1) { // Keyboard: vk or scan code; no Unicode/system commands.
    return !r.x && !r.y && r.data>0 && r.data<=255 && !(r.flags&~0x0bU);
  }
  if(r.type!=0 || !r.flags || (r.flags&~0xe9ffU)) return false;
  if(r.flags&0x180U) { if((r.data!=1 && r.data!=2) || (r.flags&0x800U)) return false; }
  else if(!(r.flags&0x800U) && r.data) return false;
  if(r.flags&0x8000U) return r.x>=0 && r.x<=65535 && r.y>=0 && r.y<=65535;
  return r.x>=-65535 && r.x<=65535 && r.y>=-65535 && r.y<=65535;
}
inline bool validFrame(const Response& r) {
  return r.signature==magic && r.status==Status::Frame && r.width>0 && r.width<=8192
    && r.height>0 && r.height<=8192 && r.stride==r.width*4
    && static_cast<std::uint64_t>(r.stride)*r.height==r.bytes && r.bytes<=maxPixelsBytes;
}
} // namespace sanser::uac
