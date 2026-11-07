#pragma once
#include "transport.h"
namespace sanser::desktop {
// Implementations schedule OS work and return promptly to UDP receive.
void platformClipboard(bool write,std::string text,std::function<bool()> current,Peer::ClipboardResult completed);
}
