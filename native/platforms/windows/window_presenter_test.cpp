#include "window_presenter.h"
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

using namespace sanser::desktop;
namespace {
void require(bool value,const char* message) { if(!value) throw std::runtime_error(message); }
struct Surface {
  HDC dc=CreateCompatibleDC(nullptr);
  HBITMAP bitmap=nullptr; HGDIOBJ previous=nullptr; std::uint8_t* pixels=nullptr;
  Surface() {
    BITMAPINFO info{}; info.bmiHeader.biSize=sizeof(BITMAPINFOHEADER); info.bmiHeader.biWidth=80; info.bmiHeader.biHeight=-80;
    info.bmiHeader.biPlanes=1; info.bmiHeader.biBitCount=32; info.bmiHeader.biCompression=BI_RGB;
    void* data=nullptr; bitmap=CreateDIBSection(dc,&info,DIB_RGB_COLORS,&data,nullptr,0); pixels=static_cast<std::uint8_t*>(data);
    if(dc && bitmap) previous=SelectObject(dc,bitmap);
  }
  ~Surface() { if(dc && previous) SelectObject(dc,previous); if(bitmap) DeleteObject(bitmap); if(dc) DeleteDC(dc); }
  bool color(unsigned x,unsigned y,std::uint8_t blue,std::uint8_t green,std::uint8_t red) const {
    const auto* p=pixels+(y*80+x)*4; return p[0]==blue && p[1]==green && p[2]==red;
  }
};
}
int main() {
  try {
    Surface target; require(target.dc && target.bitmap && target.pixels,"Unable to create offscreen test surface");
    std::vector<std::uint8_t> image(4*2*4,0);
    for(unsigned y=0;y<2;++y) for(unsigned x=0;x<4;++x) {
      const auto offset=(y*4+x)*4; image[offset]=y ? 200 : 20; image[offset+1]=70; image[offset+2]=110; image[offset+3]=255;
    }
    // GDI lazily creates process-wide brush/font caches on the first paint.
    // Warm both image and waiting-text paths before measuring owned handles.
    {
      WindowPresenter warmup; RECT video{};
      require(warmup.paint(target.dc,80,80,image.data(),4,2,video),"Warmup image paint failed");
      require(warmup.paint(target.dc,80,80,nullptr,0,0,video),"Warmup text paint failed");
    }
    GdiFlush();
    const auto handles=GetGuiResources(GetCurrentProcess(),GR_GDIOBJECTS);
    {
      WindowPresenter presenter; RECT video{};
      for(unsigned i=0;i<40;++i) {
        const int size=i%2 ? 64 : 80;
        require(presenter.paint(target.dc,size,size,image.data(),4,2,video),"Unable to present decoded image");
        GdiFlush();
        require(video.left==0 && video.right==size && video.top==size/4 && video.bottom==size*3/4,"Aspect ratio changed during resize");
        require(target.color(2,2,0,0,0),"Letterbox was not cleared in the completed frame");
        require(target.color(size/2,size/4+2,20,70,110),"Top-down image orientation or color changed");
        require(target.color(size/2,size*3/4-2,200,70,110),"Bottom image content was lost");
      }
      require(presenter.paint(target.dc,0,0,image.data(),4,2,video),"Minimized window paint failed");
      require(video.right==0 && video.bottom==0,"Minimized window retained stale input bounds");
      require(presenter.paint(target.dc,80,80,nullptr,0,0,video),"Waiting screen paint failed");
    }
    GdiFlush();
    const auto after=GetGuiResources(GetCurrentProcess(),GR_GDIOBJECTS);
    if(after!=handles) throw std::runtime_error("GDI handles before="+std::to_string(handles)+" after="+std::to_string(after));
    std::cout<<"Windows presenter: frame colors, orientation, letterbox, resize, minimize and GDI cleanup verified\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
