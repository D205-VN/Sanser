#include "ime_panel.h"
#include <iostream>
#include <memory>
#include <stdexcept>
#include <vector>
using namespace sanser::desktop;
namespace {
void require(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
void pump() { MSG message{}; for(unsigned i=0;i<256 && PeekMessageW(&message,nullptr,0,0,PM_REMOVE);++i) { TranslateMessage(&message); DispatchMessageW(&message); } }
struct Fixture {
  HWND owner=nullptr;
  bool available=true,focused=true,accepted=true;
  std::uint32_t epoch=1;
  unsigned begins=0;
  std::vector<std::pair<std::string,std::uint32_t>> commits;
  std::unique_ptr<WindowsTextInput> input;
  Fixture() {
    WNDCLASSW klass{}; klass.lpfnWndProc=DefWindowProcW; klass.hInstance=GetModuleHandleW(nullptr); klass.lpszClassName=L"SanserImeRegressionOwner";
    RegisterClassW(&klass);
    owner=CreateWindowW(klass.lpszClassName,L"IME regression",WS_OVERLAPPEDWINDOW,0,0,800,600,nullptr,nullptr,klass.hInstance,nullptr);
    require(owner!=nullptr,"native owner creation failed");
    input=std::make_unique<WindowsTextInput>(owner,WindowsTextInput::Callbacks{
      [&] { return available; },[&] { return focused; },[&] { return epoch; },
      [&](const std::string& value,std::uint32_t generation) { commits.emplace_back(value,generation); return accepted; },[&] { ++begins; }
    });
  }
  ~Fixture() { input.reset(); if(IsWindow(owner)) DestroyWindow(owner); pump(); }
  void open() { require(input->open(),"native text draft did not open"); }
  void text(const wchar_t* value) { SetWindowTextW(input->editor(),value); input->refresh(); }
  void send() { SendMessageW(input->panel(),WM_COMMAND,MAKEWPARAM(WindowsTextInput::sendId,BN_CLICKED),reinterpret_cast<LPARAM>(GetDlgItem(input->panel(),WindowsTextInput::sendId))); }
};
void testConsentAndComposition() {
  Fixture f; f.available=false; f.input->refresh();
  require(!f.input->open() && f.begins==0,"unsupported host enabled input or reset");
  require(GetMenuState(GetSystemMenu(f.owner,FALSE),WindowsTextInput::menuCommand,MF_BYCOMMAND)&MF_GRAYED,"unsupported system-menu action enabled");
  f.available=true; f.open();
  require(f.begins==1 && f.input->owns(f.input->editor()),"editor ownership/reset missing");
  f.text(L"tiee");
  SendMessageW(f.input->editor(),WM_IME_STARTCOMPOSITION,0,0);
  f.text(L"tiế");
  require(!IsWindowEnabled(GetDlgItem(f.input->panel(),WindowsTextInput::sendId)),"Send active during IME candidate");
  f.send(); require(f.commits.empty(),"intermediate IME candidate reached network");
  require(SendMessageW(f.input->editor(),WM_GETDLGCODE,VK_RETURN,0)&DLGC_WANTALLKEYS,"candidate Enter would become dialog Send");
  f.text(L"tiếng e\u0301 😀\r\n中文");
  SendMessageW(f.input->editor(),WM_IME_ENDCOMPOSITION,0,0); pump();
  require(IsWindowEnabled(GetDlgItem(f.input->panel(),WindowsTextInput::sendId)),"final draft cannot be submitted");
  f.send();
  require(!f.input->isOpen() && f.commits.size()==1 && f.commits[0]==std::make_pair(std::string("tiếng e\xcc\x81 😀\n中文"),std::uint32_t{1}),"Unicode commit altered text, generation or newline");
  pump(); require(f.commits.size()==1,"closed draft automatically replayed");
}
void testBoundsFailureAndSelection() {
  Fixture f; f.open();
  f.text(std::wstring(257,L'a').c_str()); f.send();
  require(f.commits.empty() && f.input->isOpen(),"oversized draft partially sent or discarded");
  f.text(std::wstring(5000,L'a').c_str()); f.send();
  require(f.commits.empty(),"WM_SETTEXT bypass of edit limit allowed a large wire edit");
  const wchar_t invalid[]={L'a',0xd800,0}; f.text(invalid); f.send();
  require(f.commits.empty(),"unpaired surrogate reached network");
  f.text(L"ế 😀"); SendMessageW(f.input->editor(),EM_SETSEL,0,1); f.accepted=false; f.send();
  DWORD start=0,end=0; SendMessageW(f.input->editor(),EM_GETSEL,reinterpret_cast<WPARAM>(&start),reinterpret_cast<LPARAM>(&end));
  require(f.input->isOpen() && f.commits.size()==1 && start==0 && end==1,"rejection lost draft/selection");
  for(unsigned i=0;i<5;++i) f.input->refresh();
  require(f.commits.size()==1,"rejected draft retried automatically");
  f.accepted=true; f.send(); require(f.commits.size()==2 && !f.input->isOpen(),"explicit retry failed");
}
void testSessionAndFocus() {
  Fixture f; f.open(); f.text(L"old generation"); ++f.epoch; f.input->refresh();
  require(!f.input->isOpen() && f.commits.empty(),"generation change retained draft");
  f.open(); f.text(L"permission lost"); f.available=false; f.send();
  require(!f.input->isOpen() && f.commits.empty(),"permission loss submitted draft");
  f.available=true; f.open(); f.text(L"focus lost");
  SendMessageW(f.input->editor(),WM_KILLFOCUS,0,0);
  // The cancel is posted to avoid destroying EDIT on its own focus stack. Even
  // a synchronous submit before the posted cancel must see the immediate fence.
  f.send(); pump();
  require(!f.input->isOpen() && f.commits.empty(),"focus-loss race submitted stale text");
  f.open(); f.text(L"button focus");
  const auto button=GetDlgItem(f.input->panel(),WindowsTextInput::sendId);
  SendMessageW(f.input->editor(),WM_KILLFOCUS,reinterpret_cast<WPARAM>(button),0);
  f.send(); require(f.commits.size()==1,"focus move inside draft canceled submission");
  f.open(); f.text(L"inactive app"); f.focused=false; f.send();
  require(!f.input->isOpen() && f.commits.size()==1,"inactive owner sent draft");
  f.focused=true; f.open(); f.text(L"cancel composition");
  SendMessageW(f.input->editor(),WM_IME_STARTCOMPOSITION,0,0); f.input->cancel(); pump();
  require(f.commits.size()==1 && !f.input->isOpen(),"cancel finalized text into network");
  f.open(); f.text(L"closing owner"); DestroyWindow(f.owner); f.input->detach();
  require(!f.input->isOpen() && f.commits.size()==1,"owner close retained input");
}
void testOldPostedCancel() {
  Fixture f; f.open();
  const auto old=f.input->panel();
  const auto oldSend=GetDlgItem(old,WindowsTextInput::sendId);
  SendMessageW(f.input->editor(),WM_KILLFOCUS,0,0);
  MSG stale{};
  require(PeekMessageW(&stale,old,WM_APP+74,WM_APP+74,PM_REMOVE)!=FALSE,"focus cancel was not queued");
  f.input->cancel(); f.open(); f.text(L"new draft");
  // Simulate delayed delivery after a native window handle is reused.
  SendMessageW(f.input->panel(),stale.message,stale.wParam,stale.lParam);
  SendMessageW(f.input->panel(),WM_COMMAND,MAKEWPARAM(WindowsTextInput::sendId,BN_CLICKED),reinterpret_cast<LPARAM>(oldSend));
  require(f.input->isOpen() && f.commits.empty(),"old posted cancellation closed a newer draft");
}
void testKeyboardSubmission() {
  struct Keyboard {
    BYTE original[256]{};
    Keyboard() { require(GetKeyboardState(original)!=FALSE,"read test-thread keyboard state"); }
    ~Keyboard() { SetKeyboardState(original); }
    void set(bool altGr=false) {
      BYTE value[256]{}; value[VK_CONTROL]=0x80;
      if(altGr) { value[VK_MENU]=value[VK_RMENU]=0x80; }
      require(SetKeyboardState(value)!=FALSE,"set test-thread keyboard state");
    }
  } keyboard;
  Fixture f; f.open(); keyboard.set(true);
  SendMessageW(f.input->editor(),WM_SYSKEYDOWN,'I',0);
  require(f.input->isOpen() && f.commits.empty(),"AltGr+I incorrectly treated as panel toggle");
  keyboard.set(); f.text(L"candidate");
  SendMessageW(f.input->editor(),WM_IME_STARTCOMPOSITION,0,0);
  SendMessageW(f.input->editor(),WM_KEYDOWN,VK_RETURN,0);
  require(f.commits.empty(),"Ctrl+Enter submitted active candidate");
  f.text(L"chữ đã chốt");
  SendMessageW(f.input->editor(),WM_IME_ENDCOMPOSITION,0,0); pump();
  SendMessageW(f.input->editor(),WM_KEYDOWN,VK_RETURN,0);
  require(f.commits.size()==1 && f.commits[0].first=="chữ đã chốt" && !f.input->isOpen(),"Ctrl+Enter did not submit exactly one final draft");
}
}
int main() {
  try { testConsentAndComposition(); testBoundsFailureAndSelection(); testSessionAndFocus(); testKeyboardSubmission(); testOldPostedCancel();
    std::cout<<"Win32 native IME draft: capability, local composition, Unicode, limits, generation, focus and no-auto-retry verified\n"; return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
