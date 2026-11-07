#pragma once
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>

namespace sanser {
// One committed keyboard edit fits in a single authenticated UDP datagram.
// Reject the entire edit before injecting anything: no replacement characters,
// overlong UTF-8, lone surrogates, NUL or terminal/control escape sequences.
inline constexpr std::size_t maxTextInputBytes=1024, maxTextInputUnits=256;
inline std::optional<std::u16string> validatedUtf16(std::string_view text,std::size_t bytes,std::size_t units,bool clipboard=false) {
  if((text.empty() && !clipboard) || text.size()>bytes) return std::nullopt;
  std::u16string result;
  for(std::size_t i=0;i<text.size();) {
    auto first=static_cast<std::uint8_t>(text[i++]);
    std::uint32_t scalar=first, minimum=0; unsigned trailing=0;
    if(first>=0xc2 && first<=0xdf) { scalar=first&31; trailing=1; minimum=0x80; }
    else if(first>=0xe0 && first<=0xef) { scalar=first&15; trailing=2; minimum=0x800; }
    else if(first>=0xf0 && first<=0xf4) { scalar=first&7; trailing=3; minimum=0x10000; }
    else if(first>=0x80) return std::nullopt;
    if(i+trailing>text.size()) return std::nullopt;
    for(unsigned n=0;n<trailing;++n) {
      const auto next=static_cast<std::uint8_t>(text[i++]);
      if((next&0xc0)!=0x80) return std::nullopt;
      scalar=(scalar<<6)|(next&63);
    }
    if(scalar<minimum || scalar>0x10ffff || (scalar>=0xd800 && scalar<=0xdfff) ||
       (scalar<0x20 && scalar!='\n' && scalar!='\t' && !(clipboard && scalar=='\r')) || (scalar>=0x7f && scalar<=0x9f)) return std::nullopt;
    if(scalar<0x10000) result.push_back(static_cast<char16_t>(scalar));
    else { scalar-=0x10000; result.push_back(static_cast<char16_t>(0xd800+(scalar>>10))); result.push_back(static_cast<char16_t>(0xdc00+(scalar&1023))); }
    if(result.size()>units) return std::nullopt;
  }
  return result;
}
inline std::optional<std::u16string> textInputUtf16(std::string_view text) { return validatedUtf16(text,maxTextInputBytes,maxTextInputUnits); }
inline constexpr std::size_t maxClipboardBytes=8192;
inline std::optional<std::u16string> clipboardUtf16(std::string_view text) { return validatedUtf16(text,maxClipboardBytes,maxClipboardBytes,true); }
}
