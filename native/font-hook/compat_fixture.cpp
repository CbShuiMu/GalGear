// Isolated regression checks for the restored runtime helpers; no game files.
#define NOMINMAX
#include <windows.h>
#include <cstdio>
#include <stdexcept>
namespace Utils { inline void Trace(const char*, ...) {} }
#include "compat/pfs.h"
#include "compat/table_scale.h"
#include "compat/krkr_cache.h"

static void Check(bool value, const char* name) { if (!value) throw std::runtime_error(name); }
static void Word(std::vector<BYTE>& bytes, DWORD value) {
    for (int i=0; i<4; ++i) bytes.push_back(static_cast<BYTE>(value >> (8*i)));
}
static void Parser() {
    const std::string name = "system/table/list_windows.tbl";
    const DWORD indexSize = static_cast<DWORD>(4 + 4 + name.size() + 12);
    std::vector<BYTE> bytes;
    Word(bytes, 1); Word(bytes, static_cast<DWORD>(name.size()));
    bytes.insert(bytes.end(), name.begin(), name.end());
    Word(bytes, 0); Word(bytes, 7+indexSize); Word(bytes, 3);
    std::vector<GalGearPfs::Entry> entries;
    Check(GalGearPfs::ParseLayout(bytes, 7+indexSize+3, false, entries), "PFS normal index");
    Check(entries.size()==1 && entries[0].name=="system\\table\\list_windows.tbl", "PFS normalized name");
    Check(!GalGearPfs::ParseLayout(bytes, 7+indexSize+2, false, entries), "PFS truncated data");
    bytes[8]=':';
    Check(!GalGearPfs::ParseLayout(bytes, 7+indexSize+3, false, entries), "PFS path rejection");
    for (auto path : {"../font", "C:/font", "/font", "a//b", "a/../b"}) {
        std::string invalid=path;
        Check(!GalGearPfs::Normalize(invalid), "PFS boundary");
    }
    GalGearPfs::Candidate base{}, overlay{}, unrelated{};
    Check(GalGearPfs::CandidateName(L"Game.pfs", L"Game", base), "base archive");
    Check(GalGearPfs::CandidateName(L"Game.pfs.003", L"Game", overlay), "overlay archive");
    Check(GalGearPfs::CandidateName(L"Other.pfs.003", L"Game", unrelated), "other archive");
    Check(GalGearPfs::Before(overlay, base) && GalGearPfs::Before(overlay, unrelated), "overlay precedence");
    Check(!GalGearPfs::CandidateName(L"Game.pfs.backup", L"Game", base), "unrecognized suffix");
}
static void Table() {
    std::string table="size=24, rubysize='10',\n-- size=99\nname='size=20', size=base*2,\n--[[ rubysize=40 ]]\n";
    Check(GalGearTable::Scale(table, 150), "table scale");
    Check(table=="size=36, rubysize='15',\n-- size=99\nname='size=20', size=base*2,\n--[[ rubysize=40 ]]\n", "table preserves expressions and comments");
    const auto unchanged=table;
    Check(!GalGearTable::Scale(table, 100) && table==unchanged, "neutral scale");
    Check(!GalGearTable::Scale(table, 301) && table==unchanged, "out of range scale");
}
static void CacheAbi() {
    auto bytes=static_cast<BYTE*>(VirtualAlloc(nullptr, 4096, MEM_RESERVE|MEM_COMMIT, PAGE_EXECUTE_READWRITE));
    Check(bytes!=nullptr, "test allocation");
    struct Memory { BYTE* bytes; ~Memory() { VirtualFree(bytes, 0, MEM_RELEASE); } } memory{bytes};
    GalGearKrkr::Image image{bytes, 4096};
    Check(!GalGearKrkr::Probe(image), "unknown ABI rejected");
#ifndef _WIN64
    const char name[]=".?AUtTVPClearFontCacheCallback@@";
    memcpy(bytes+72, name, sizeof(name));
    DWORD col[]={0,0,0,reinterpret_cast<DWORD>(bytes+64),0};
    memcpy(bytes+256,col,sizeof(col));
    DWORD slot[]={reinterpret_cast<DWORD>(bytes+256),reinterpret_cast<DWORD>(bytes+512)};
    memcpy(bytes+384,slot,sizeof(slot));
    const BYTE code[]={0x55,0x8b,0xec,0x83,0x7d,0x0c,0x0f,0x7c,0x06,0x5d,0xe9,0,0,0,0,0x5d,0xc3};
    memcpy(bytes+512,code,sizeof(code));
    int displacement=768-527;
    memcpy(bytes+523,&displacement,4);
    Check(GalGearKrkr::Probe(image)==reinterpret_cast<GalGearKrkr::Clear>(bytes+768), "known ABI resolved");
    bytes[512]=0x90;
    Check(!GalGearKrkr::Probe(image), "changed ABI rejected");
#endif
}
int main() {
    try { Parser(); Table(); CacheAbi(); puts("Compatibility helper checks passed"); return 0; }
    catch (const std::exception& error) { fprintf(stderr,"%s\n",error.what()); return 1; }
}
