#include "window_presenter.h"
#include <algorithm>

namespace sanser::desktop {
WindowPresenter::~WindowPresenter() {
  if(buffer_ && original_) SelectObject(buffer_,original_);
  if(bitmap_) DeleteObject(bitmap_);
  if(buffer_) DeleteDC(buffer_);
}
bool WindowPresenter::resize(HDC target,int width,int height) {
  if(!buffer_) { buffer_=CreateCompatibleDC(target); if(!buffer_) return false; }
  if(bitmap_ && width==width_ && height==height_) return true;
  auto next=CreateCompatibleBitmap(target,width,height);
  if(!next) return false;
  auto previous=SelectObject(buffer_,next);
  if(!previous || previous==HGDI_ERROR) { DeleteObject(next); return false; }
  if(!original_) original_=previous;
  if(bitmap_) DeleteObject(bitmap_);
  bitmap_=next; width_=width; height_=height;
  return true;
}
bool WindowPresenter::paint(HDC target,int width,int height,const std::uint8_t* pixels,
                            std::uint32_t sourceWidth,std::uint32_t sourceHeight,RECT& video) {
  if(width<=0 || height<=0) { video={}; return true; }
  if(!resize(target,width,height)) return false;
  RECT area{0,0,width,height};
  FillRect(buffer_,&area,static_cast<HBRUSH>(GetStockObject(BLACK_BRUSH)));
  if(pixels && sourceWidth && sourceHeight) {
    const auto scale=std::min(static_cast<double>(width)/sourceWidth,static_cast<double>(height)/sourceHeight);
    const auto drawnWidth=std::max(1L,static_cast<LONG>(sourceWidth*scale));
    const auto drawnHeight=std::max(1L,static_cast<LONG>(sourceHeight*scale));
    const auto x=(width-drawnWidth)/2,y=(height-drawnHeight)/2;
    video={x,y,x+drawnWidth,y+drawnHeight};
    BITMAPINFO info{}; info.bmiHeader.biSize=sizeof(BITMAPINFOHEADER);
    info.bmiHeader.biWidth=static_cast<LONG>(sourceWidth); info.bmiHeader.biHeight=-static_cast<LONG>(sourceHeight);
    info.bmiHeader.biPlanes=1; info.bmiHeader.biBitCount=32; info.bmiHeader.biCompression=BI_RGB;
    SetStretchBltMode(buffer_,COLORONCOLOR);
    if(StretchDIBits(buffer_,x,y,drawnWidth,drawnHeight,0,0,sourceWidth,sourceHeight,pixels,&info,DIB_RGB_COLORS,SRCCOPY)<=0) return false;
  } else {
    video={};
    SetTextColor(buffer_,RGB(210,220,230)); SetBkMode(buffer_,TRANSPARENT);
    DrawTextW(buffer_,L"Waiting for the remote desktop…",-1,&area,DT_CENTER|DT_VCENTER|DT_SINGLELINE);
  }
  // Never erase the visible DC before the frame is ready: that exposes a
  // black flash between FillRect and a slow software stretch on Windows.
  return BitBlt(target,0,0,width,height,buffer_,0,0,SRCCOPY)!=FALSE;
}
}
