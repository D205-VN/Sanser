#pragma once
#include "text_input.h"
#include <cstdio>
#include <optional>
#include <string>
#include <string_view>
#include <vector>
#include <cstdint>

namespace sanser::desktop {
inline std::string userHex(std::string_view text) {
  static constexpr char hex[]="0123456789abcdef";
  std::string out;out.reserve(text.size()*2);
  for(unsigned char c:text) {out.push_back(hex[c>>4]);out.push_back(hex[c&15]);}return out;
}
inline std::optional<std::string> userUnhex(std::string_view text) {
  if(text.size()%2 || text.size()>sanser::maxClipboardBytes*2) return std::nullopt;
  const auto nibble=[](char c)->int { if(c>='0'&&c<='9')return c-'0';if(c>='a'&&c<='f')return c-'a'+10;return -1; };
  std::string out;out.reserve(text.size()/2);
  for(std::size_t i=0;i<text.size();i+=2) {const auto a=nibble(text[i]),b=nibble(text[i+1]);if(a<0 || b<0)return std::nullopt;out.push_back(static_cast<char>((a<<4)|b));}return out;
}
struct UserCommand { enum Kind {Text,ReadClipboard,WriteClipboard,ReadLocalClipboard,WriteLocalClipboard,FileMessage} kind;std::string request,body; };
inline std::optional<std::vector<std::uint8_t>> userFileBody(const std::string& body,std::uint32_t generation) {
  if(body.size()<25 || body.size()>8192)return std::nullopt;
  std::uint32_t epoch=0;for(unsigned i=0;i<4;++i)epoch=(epoch<<8)|static_cast<std::uint8_t>(body[i]);
  if(!generation || epoch!=generation)return std::nullopt;
  return std::vector<std::uint8_t>(body.begin()+4,body.end());
}
inline std::optional<UserCommand> parseUserCommand(std::string_view line) {
  UserCommand command;
  if(line.starts_with("user-text\t")) {command.kind=UserCommand::Text;line.remove_prefix(10);}
  else if(line.starts_with("clipboard-read\t")) {command.kind=UserCommand::ReadClipboard;line.remove_prefix(15);}
  else if(line.starts_with("clipboard-write\t")) {command.kind=UserCommand::WriteClipboard;line.remove_prefix(16);}
  else if(line.starts_with("clipboard-local-read\t")) {command.kind=UserCommand::ReadLocalClipboard;line.remove_prefix(21);}
  else if(line.starts_with("clipboard-local-write\t")) {command.kind=UserCommand::WriteLocalClipboard;line.remove_prefix(22);}
  else if(line.starts_with("file-send\t")) {command.kind=UserCommand::FileMessage;line.remove_prefix(10);}
  else return std::nullopt;
  if(line.size()<36) return std::nullopt;
  for(unsigned i=0;i<36;++i) {
    const char c=line[i];const bool dash=i==8 || i==13 || i==18 || i==23;
    if(dash ? c!='-' : !((c>='0' && c<='9') || (c>='a' && c<='f'))) return std::nullopt;
  }
  command.request=line.substr(0,36);line.remove_prefix(36);
  if(command.kind==UserCommand::ReadClipboard || command.kind==UserCommand::ReadLocalClipboard) return line.empty() ? std::optional<UserCommand>{std::move(command)}:std::nullopt;
  if(line.empty() || line[0]!='\t') return std::nullopt;
  const auto body=userUnhex(line.substr(1));if(!body)return std::nullopt;
  command.body=*body;return command;
}
// stdout is a private reply channel. The desktop shell strips these records
// before any error tail, diagnostic collector or debug log sees their payload.
inline void userReply(const std::string& request,unsigned status,const std::string& body={}) {
  const auto encoded=userHex(body);
  std::fprintf(stdout,"SNUSER_REPLY\t%s\t%u\t%s\n",request.c_str(),status,encoded.c_str());std::fflush(stdout);
}
}
