#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include "clipboard.h"
#include "text_input.h"
#include <cstring>
namespace sanser::desktop {
void platformClipboard(bool write,std::string text,std::function<bool()> current,Peer::ClipboardResult completed) {
  std::thread([write,text=std::move(text),current=std::move(current),completed=std::move(completed)] {
    try {
      if(!current()) {completed(5,{});return;}
      HWND owner=CreateWindowExW(0,L"STATIC",L"Sanser clipboard",0,0,0,0,0,HWND_MESSAGE,nullptr,GetModuleHandleW(nullptr),nullptr);
      if(!owner) {completed(3,{});return;}
      struct Window {HWND value;~Window(){DestroyWindow(value);}} window{owner};
      if(!OpenClipboard(owner)) {completed(2,{});return;}
      struct Close {~Close(){CloseClipboard();}} close;
      if(!current()) {completed(5,{});return;}
      if(write) {
        const auto value=sanser::clipboardUtf16(text);if(!value) {completed(4,{});return;}
        HGLOBAL memory=GlobalAlloc(GMEM_MOVEABLE,(value->size()+1)*sizeof(wchar_t));
        if(!memory) {completed(3,{});return;}
        auto* target=static_cast<wchar_t*>(GlobalLock(memory));
        if(!target) {GlobalFree(memory);completed(3,{});return;}
        std::memcpy(target,value->data(),value->size()*sizeof(char16_t));target[value->size()]=0;GlobalUnlock(memory);
        if(!EmptyClipboard() || !SetClipboardData(CF_UNICODETEXT,memory)) {GlobalFree(memory);completed(3,{});return;}
        // Ownership of the materialized HGLOBAL belongs to the OS now.
        completed(0,{});
      } else {
        HANDLE memory=GetClipboardData(CF_UNICODETEXT);if(!memory) {completed(3,{});return;}
        const auto size=GlobalSize(memory);
        if(size<sizeof(wchar_t) || size>(sanser::maxClipboardBytes+1)*sizeof(wchar_t)) {completed(4,{});return;}
        const auto* source=static_cast<const wchar_t*>(GlobalLock(memory));if(!source) {completed(3,{});return;}
        struct Unlock {HANDLE value;~Unlock(){GlobalUnlock(value);}} unlock{memory};
        const auto* end=std::find(source,source+size/sizeof(wchar_t),L'\0');
        if(end==source+size/sizeof(wchar_t)) {completed(4,{});return;}
        const auto units=static_cast<int>(end-source);
        if(!units) {completed(0,{});return;}
        const auto count=WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,source,units,nullptr,0,nullptr,nullptr);
        if(count<=0 || count>sanser::maxClipboardBytes) {completed(4,{});return;}
        std::string result(count,'\0');
        if(WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,source,units,result.data(),count,nullptr,nullptr)!=count || !sanser::clipboardUtf16(result)) {completed(4,{});return;}
        completed(0,std::move(result));
      }
    } catch(...) {completed(3,{});}
  }).detach();
}
}
