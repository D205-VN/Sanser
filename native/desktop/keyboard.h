#pragma once
#include <array>
#include <cstdint>
namespace sanser::desktop {
// USB HID keyboard usage -> macOS virtual key, and Windows set-1 scan code.
// Extended Windows scan codes carry bit 8. Zero means unsupported.
struct KeyMapping { std::uint16_t hid, mac, windows; };
inline constexpr KeyMapping keys[] = {
 {4,0,0x1e},{5,11,0x30},{6,8,0x2e},{7,2,0x20},{8,14,0x12},{9,3,0x21},
 {10,5,0x22},{11,4,0x23},{12,34,0x17},{13,38,0x24},{14,40,0x25},{15,37,0x26},
 {16,46,0x32},{17,45,0x31},{18,31,0x18},{19,35,0x19},{20,12,0x10},{21,15,0x13},
 {22,1,0x1f},{23,17,0x14},{24,32,0x16},{25,9,0x2f},{26,13,0x11},{27,7,0x2d},{28,16,0x15},{29,6,0x2c},
 {30,18,0x02},{31,19,0x03},{32,20,0x04},{33,21,0x05},{34,23,0x06},{35,22,0x07},{36,26,0x08},{37,28,0x09},{38,25,0x0a},{39,29,0x0b},
 {40,36,0x1c},{41,53,0x01},{42,51,0x0e},{43,48,0x0f},{44,49,0x39},{45,27,0x0c},{46,24,0x0d},{47,33,0x1a},{48,30,0x1b},{49,42,0x2b},
 {51,41,0x27},{52,39,0x28},{53,50,0x29},{54,43,0x33},{55,47,0x34},{56,44,0x35},{57,57,0x3a},
 {58,122,0x3b},{59,120,0x3c},{60,99,0x3d},{61,118,0x3e},{62,96,0x3f},{63,97,0x40},{64,98,0x41},{65,100,0x42},{66,101,0x43},{67,109,0x44},{68,103,0x57},{69,111,0x58},
 {73,114,0x152},{74,115,0x147},{75,116,0x149},{76,117,0x153},{77,119,0x14f},{78,121,0x151},{79,124,0x14d},{80,123,0x14b},{81,125,0x150},{82,126,0x148},
 {224,59,0x1d},{225,56,0x2a},{226,58,0x38},{227,55,0x15b},{228,62,0x11d},{229,60,0x36},{230,61,0x138},{231,54,0x15c}
};
inline int macKey(std::uint16_t hid) { for(auto k:keys) if(k.hid==hid) return k.mac; return -1; }
inline std::uint16_t hidFromMac(std::uint16_t code) { for(auto k:keys) if(k.mac==code) return k.hid; return 0; }
inline std::uint16_t windowsKey(std::uint16_t hid) { for(auto k:keys) if(k.hid==hid) return k.windows; return 0; }
inline std::uint16_t hidFromWindows(std::uint16_t code) { for(auto k:keys) if(k.windows==code) return k.hid; return 0; }
}
