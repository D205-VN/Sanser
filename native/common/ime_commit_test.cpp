#include "ime_commit.h"
#include <stdexcept>
#include <iostream>
int main() {
  try {
    const auto require=[](bool value) { if(!value) throw std::runtime_error("IME draft conversion mismatch"); };
    require(sanser::imeCommitUtf8(u"Tiếng Việt e\u0301 😀 中文")=="Tiếng Việt e\xcc\x81 😀 中文");
    require(sanser::imeCommitUtf8(u"a\r\nb\n\t")=="a\nb\n\t");
    require(sanser::imeCommitUtf8(std::u16string(256,u'a')).has_value());
    require(!sanser::imeCommitUtf8(std::u16string(257,u'a')));
    std::u16string newlines; for(unsigned i=0;i<256;++i) newlines+=u"\r\n";
    require(sanser::imeCommitUtf8(newlines)==std::string(256,'\n'));
    newlines+=u"\r\n"; require(!sanser::imeCommitUtf8(newlines));
    std::u16string emoji; for(unsigned i=0;i<128;++i) emoji+=u"😀";
    require(sanser::imeCommitUtf8(emoji).has_value()); emoji+=u"😀"; require(!sanser::imeCommitUtf8(emoji));
    for(const auto& value: {std::u16string{},std::u16string{0xd800},std::u16string{0xdc00},std::u16string{u'a',0xd800,u'b'},std::u16string{u'a',0,u'b'},std::u16string{u'a',27},std::u16string{127},std::u16string{159},std::u16string{u'\r'}}) require(!sanser::imeCommitUtf8(value));
    std::cout<<"IME UTF16: strict Unicode, CRLF, unit bounds and no partial conversion verified\n";
    return 0;
  } catch(const std::exception& error) { std::cerr<<error.what()<<'\n'; return 1; }
}
