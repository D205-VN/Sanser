#pragma once
#include <optional>
#include <string>
#include <string_view>
namespace sanser::desktop {
struct ExternalVoiceCommand { std::string request; bool active; };
inline std::optional<ExternalVoiceCommand> externalVoiceCommand(std::string_view line) {
  constexpr std::string_view prefix="voice-external\t";
  if(!line.starts_with(prefix)) return {};
  line.remove_prefix(prefix.size());
  if(line.size()!=38 || line[36]!='\t' || (line[37]!='0' && line[37]!='1')) return {};
  for(unsigned i=0;i<36;++i) {
    const auto c=line[i];
    if(i==8 || i==13 || i==18 || i==23) { if(c!='-') return {}; }
    else if(!((c>='0'&&c<='9') || (c>='a'&&c<='f') || (c>='A'&&c<='F'))) return {};
  }
  return ExternalVoiceCommand{std::string(line.substr(0,36)),line[37]=='1'};
}
bool externalVoiceActive();
void externalVoiceActive(bool active);
}
