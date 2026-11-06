#pragma once
#include <cstdint>
namespace GalGearFont {
inline constexpr wchar_t ClassName[] = L"GalGear.FontHook.Bridge.v2";
inline constexpr uintptr_t PacketId = 0x47474632;
inline constexpr unsigned QueryMessage = 0x8000 + 0x476;
inline constexpr uintptr_t Capabilities = 0x000200ff;
struct Packet {
    uint32_t version, length, height, width;
    wchar_t face[32];
    int32_t spacing, line;
    uint32_t weight, vertical;
    int32_t ascent, descent;
    uint32_t spoof, from, to, mapping, codepage, reserved;
};
static_assert(sizeof(Packet) == 128);
inline bool Charset(uint32_t c) { return c == 1 || c == 128 || c == 129 || c == 134 || c == 136; }
inline bool Valid(const Packet& p) {
    if (p.version != 2 || p.length != sizeof(p) || p.height < 50 || p.height > 300 ||
        p.width < 50 || p.width > 300 || p.face[31] || p.reserved ||
        p.spacing < -20 || p.spacing > 60 || p.line < -80 || p.line > 200 ||
        p.weight < 100 || p.weight > 900 || p.vertical > 1 || p.spoof > 1 || p.mapping > 3 ||
        (p.vertical && (p.ascent < 100 || p.ascent > 2000 || p.descent < -2000 || p.descent > -1)) ||
        (p.spoof && (!Charset(p.from) || !Charset(p.to))) ||
        ((p.vertical || p.spoof) && !p.face[0]) ||
        (p.codepage != 0 && p.codepage != 932 && p.codepage != 936 && p.codepage != 949 && p.codepage != 950 && p.codepage != 65001)) return false;
    for (const wchar_t* c = p.face; *c; ++c) {
        if (*c < 32 || (*c >= 0x7f && *c <= 0x9f)) return false;
        if (*c >= 0xd800 && *c <= 0xdbff) { ++c; if (*c < 0xdc00 || *c > 0xdfff) return false; }
        else if (*c >= 0xdc00 && *c <= 0xdfff) return false;
    }
    return true;
}
}
