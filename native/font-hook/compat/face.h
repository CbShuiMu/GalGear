#pragma once
#include "font/font_patcher.h"
#include "galgear_protocol.h"
#include <string>
#include <vector>
#include <memory>

// GDI may silently ignore a Chinese family when a Japanese game requests
// SHIFTJIS_CHARSET. Keep that charset (and ANSI decoding) intact; expose a
// private, uniquely named copy with the missing OS/2 capability bit instead.
// Called only by the picker/IPC thread, which upstream excludes from hooks.
namespace GalGearFace {
inline bool Failed(const char* stage) {
    Utils::Trace("[GalGearFace] rejected stage=%s acp=%u error=%lu", stage, GetACP(), GetLastError());
    return false;
}
struct Font {
    HDC dc = CreateCompatibleDC(nullptr);
    HFONT font = nullptr;
    HGDIOBJ old = nullptr;
    Font(const wchar_t* face, BYTE charset) {
        LOGFONTW logical{};
        logical.lfHeight = -20;
        logical.lfCharSet = charset;
        wcscpy_s(logical.lfFaceName, face);
        font = CreateFontIndirectW(&logical);
        if (dc && font) old = SelectObject(dc, font);
    }
    ~Font() {
        if (old && old != HGDI_ERROR) SelectObject(dc, old);
        if (font) DeleteObject(font);
        if (dc) DeleteDC(dc);
    }
    bool name(wchar_t (&value)[LF_FACESIZE]) const {
        return old && old != HGDI_ERROR && GetTextFaceW(dc, LF_FACESIZE, value) > 0;
    }
    Font(const Font&) = delete;
    Font& operator=(const Font&) = delete;
};
struct Retained {
    std::wstring source, alias;
    HANDLE resource;
    size_t bytes;
    GalGearFont::Packet settings;
};
struct Resource {
    HANDLE value;
    ~Resource() { if (value) RemoveFontMemResourceEx(value); }
};

inline bool HasFamilyName(HDC dc, const wchar_t* requested) {
    // GDI returns English family names under a Japanese locale even when the
    // request used a Chinese alias. Validate against the font's Unicode names.
    constexpr DWORD NameTable = 0x656d616e;
    DWORD size = GetFontData(dc, NameTable, 0, nullptr, 0);
    if (size == GDI_ERROR || size < 6 || size > 1024 * 1024) return false;
    std::vector<BYTE> data(size);
    if (GetFontData(dc, NameTable, 0, data.data(), size) != size) return false;
    auto word = [&](size_t offset) { return (data[offset] << 8) | data[offset + 1]; };
    const size_t count = word(2), strings = word(4);
    if (count > (size - 6) / 12 || strings > size) return false;
    for (size_t i=0; i<count; ++i) {
        const size_t record=6+i*12;
        const int platform=word(record), kind=word(record+6);
        if ((platform!=0 && platform!=3) || (kind!=1 && kind!=4 && kind!=16)) continue;
        const size_t length=word(record+8), offset=strings+word(record+10);
        if (length%2 || length/2 >= LF_FACESIZE || offset>size || length>size-offset) continue;
        wchar_t name[LF_FACESIZE]{};
        for (size_t j=0; j<length/2; ++j) name[j]=static_cast<wchar_t>(word(offset+j*2));
        if (_wcsicmp(name, requested)==0) return true;
    }
    return false;
}

inline bool SameCloneSettings(const GalGearFont::Packet& a, const GalGearFont::Packet& b) {
    return a.vertical == b.vertical && (!a.vertical ||
        (a.ascent == b.ascent && a.descent == b.descent && a.line == b.line)) &&
        a.spoof == b.spoof && (!a.spoof || a.to == b.to);
}

// Prepare every requested font-table change before publishing Config. The private
// copy remains in memory; FontPatcher owns the actual table algorithms.
inline bool Resolve(const GalGearFont::Packet& settings, std::wstring& wide, std::string& ansi, bool& found) {
    const wchar_t* requested = settings.face;
    found = false;
    wide = requested;
    ansi.clear();
    if (!*requested) return true;
    // Old HFONT/engine objects can still reference a previously selected face.
    // Retain resources until process exit, bounded and reused per source family.
    static std::vector<Retained> retained;
    static size_t retainedBytes = 0;
    for (const auto& entry : retained) {
        if (_wcsicmp(entry.source.c_str(), requested) == 0 && SameCloneSettings(entry.settings, settings)) {
            found = true;
            wide = entry.alias;
            for (wchar_t c : wide) ansi.push_back(static_cast<char>(c)); // ASCII alias.
            return true;
        }
    }
    std::unique_ptr<Font> source;
    // DEFAULT_CHARSET follows the emulated ACP and may already select the wrong
    // font. Probe explicit charsets, accepting only the requested physical face.
    for (BYTE charset : {DEFAULT_CHARSET, ANSI_CHARSET, GB2312_CHARSET, SHIFTJIS_CHARSET,
            CHINESEBIG5_CHARSET, HANGEUL_CHARSET, JOHAB_CHARSET, GREEK_CHARSET,
            TURKISH_CHARSET, HEBREW_CHARSET, ARABIC_CHARSET, BALTIC_CHARSET,
            RUSSIAN_CHARSET, THAI_CHARSET, EASTEUROPE_CHARSET, VIETNAMESE_CHARSET, SYMBOL_CHARSET}) {
        auto candidate=std::make_unique<Font>(requested, charset);
        if (candidate->old && candidate->old!=HGDI_ERROR && HasFamilyName(candidate->dc, requested)) {
            source=std::move(candidate);
            break;
        }
    }
    if (!source) return Failed("source-family");
    found = true;
    wchar_t actual[LF_FACESIZE]{};
    if (!source->name(actual)) return Failed("source-probe");
    const std::string english = Utils::GetFontEnglishName(source->font);
    Font japanese(requested, SHIFTJIS_CHARSET);
    wchar_t mapped[LF_FACESIZE]{};
    if (!settings.vertical && !settings.spoof && japanese.name(mapped) && _wcsicmp(actual, mapped) == 0) {
        wide = actual;
        if (!english.empty() && english.size() < LF_FACESIZE) ansi = english;
        else {
            char name[LF_FACESIZE]{};
            BOOL substituted = FALSE;
            if (!WideCharToMultiByte(CP_ACP, WC_NO_BEST_FIT_CHARS, requested, -1,
                    name, LF_FACESIZE, nullptr, &substituted) || substituted) return false;
            ansi = name;
        }
        return true;
    }
    constexpr size_t MaxFontBytes = 64 * 1024 * 1024;
    constexpr size_t MaxRetainedBytes = 128 * 1024 * 1024;
    DWORD size = GetFontData(source->dc, 0, 0, nullptr, 0);
    if (size == GDI_ERROR || !size || size > MaxFontBytes || retained.size() >= 64) return Failed("source-data-limit");
    std::vector<BYTE> data(size);
    if (GetFontData(source->dc, 0, 0, data.data(), size) != size ||
        !FontPatcher::NormalizeGdiFontData(data)) return Failed("source-data");
    wchar_t alias[LF_FACESIZE]{};
    swprintf_s(alias, L"GGF_%08lX_%02u", GetCurrentProcessId(), static_cast<unsigned>(retained.size()));
    if (settings.vertical && !FontPatcher::PatchVerticalMetrics(data, settings.ascent, settings.descent, settings.line))
        return Failed("vertical-metrics");
    if (settings.spoof && !FontPatcher::PatchOS2CodePageRangeForCharset(data, settings.to))
        return Failed("charset-capability");
    if (!FontPatcher::PatchOS2CodePageRangeForCharset(data, SHIFTJIS_CHARSET) ||
        !FontPatcher::PatchNameTableFamily(data, alias) || data.size() > MaxRetainedBytes - retainedBytes) return Failed("private-font-data");
    DWORD count = 0;
    Resource resource{AddFontMemResourceEx(data.data(), static_cast<DWORD>(data.size()), nullptr, &count)};
    if (!resource.value || !count) return Failed("private-font-register");
    bool matches = false;
    {
        Font probe(alias, SHIFTJIS_CHARSET);
        wchar_t selected[LF_FACESIZE]{};
        matches = probe.name(selected) && _wcsicmp(selected, alias) == 0;
    }
    if (!matches) return Failed("private-font-probe");
    retained.push_back({requested, alias, resource.value, data.size(), settings});
    resource.value = nullptr; // Ownership retained until process exit.
    retainedBytes += data.size();
    wide = alias;
    for (wchar_t c : wide) ansi.push_back(static_cast<char>(c)); // ASCII alias.
    return true;
}
}
