#pragma once
#include "text_input.h"

namespace sanser {
// Native Windows EDIT uses CRLF. Normalize only those line endings, preserving
// every Unicode scalar/combining mark. Reject the whole draft before injection.
inline std::optional<std::string> imeCommitUtf8(std::u16string_view source) {
  if(source.empty() || source.size()>maxTextInputUnits*2) return std::nullopt;
  std::string result;
  for(std::size_t i=0;i<source.size();++i) {
    std::uint32_t scalar=source[i];
    if(scalar=='\r') {
      if(i+1>=source.size() || source[i+1]!='\n') return std::nullopt;
      ++i; scalar='\n';
    } else if(scalar>=0xd800 && scalar<=0xdbff) {
      if(i+1>=source.size() || source[i+1]<0xdc00 || source[i+1]>0xdfff) return std::nullopt;
      scalar=0x10000+((scalar-0xd800)<<10)+(source[++i]-0xdc00);
    } else if(scalar>=0xdc00 && scalar<=0xdfff) return std::nullopt;
    if(scalar<0x80) result.push_back(static_cast<char>(scalar));
    else if(scalar<0x800) { result.push_back(static_cast<char>(0xc0|(scalar>>6))); result.push_back(static_cast<char>(0x80|(scalar&63))); }
    else if(scalar<0x10000) { result.push_back(static_cast<char>(0xe0|(scalar>>12))); result.push_back(static_cast<char>(0x80|((scalar>>6)&63))); result.push_back(static_cast<char>(0x80|(scalar&63))); }
    else { result.push_back(static_cast<char>(0xf0|(scalar>>18))); result.push_back(static_cast<char>(0x80|((scalar>>12)&63))); result.push_back(static_cast<char>(0x80|((scalar>>6)&63))); result.push_back(static_cast<char>(0x80|(scalar&63))); }
  }
  return textInputUtf16(result) ? std::optional<std::string>{std::move(result)}:std::nullopt;
}
}
