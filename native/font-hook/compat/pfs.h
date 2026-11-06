#pragma once
// Bounded, read-only PFS index/resource access shared by the adapter and fixtures.
#include <windows.h>
#include <wincrypt.h>
#include <algorithm>
#include <cstdint>
#include <string>
#include <vector>

namespace GalGearPfs {
inline thread_local unsigned internalReads = 0;
struct ReadScope {
    ReadScope() { ++internalReads; }
    ~ReadScope() { --internalReads; }
    ReadScope(const ReadScope&) = delete;
    ReadScope& operator=(const ReadScope&) = delete;
};
inline bool IsReading() { return internalReads != 0; }
constexpr DWORD MaxIndex = 64 * 1024 * 1024;
constexpr DWORD MaxResource = 64 * 1024 * 1024;
struct Entry { std::string name; DWORD offset, size; };
struct Archive {
    std::wstring path;
    std::vector<Entry> entries;
    BYTE key[20]{};
    bool encrypted = false;
    DWORD indexBytes = 0;
};
struct Handle {
    HANDLE value;
    ~Handle() { if (value && value != INVALID_HANDLE_VALUE) CloseHandle(value); }
};
inline DWORD Word(const BYTE* p) {
    return DWORD(p[0]) | DWORD(p[1]) << 8 | DWORD(p[2]) << 16 | DWORD(p[3]) << 24;
}
inline bool Read(HANDLE file, uint64_t offset, void* bytes, DWORD size) {
    LARGE_INTEGER position{}; position.QuadPart = static_cast<LONGLONG>(offset);
    DWORD read = 0;
    return SetFilePointerEx(file, position, nullptr, FILE_BEGIN) &&
        ReadFile(file, bytes, size, &read, nullptr) && read == size;
}
inline bool Hash(const std::vector<BYTE>& bytes, BYTE (&key)[20]) {
    HCRYPTPROV provider = 0; HCRYPTHASH hash = 0; DWORD size = 20;
    bool ok = CryptAcquireContextW(&provider, nullptr, nullptr, PROV_RSA_FULL, CRYPT_VERIFYCONTEXT) &&
        CryptCreateHash(provider, CALG_SHA1, 0, 0, &hash) &&
        CryptHashData(hash, bytes.data(), static_cast<DWORD>(bytes.size()), 0) &&
        CryptGetHashParam(hash, HP_HASHVAL, key, &size, 0) && size == 20;
    if (hash) CryptDestroyHash(hash);
    if (provider) CryptReleaseContext(provider, 0);
    return ok;
}
inline bool Normalize(std::string& name) {
    if (name.empty() || name.front() == '/' || name.front() == '\\') return false;
    for (char& c : name) {
        if (c == ':' || static_cast<unsigned char>(c) < 32) return false;
        if (c == '/') c = '\\';
        if (c >= 'A' && c <= 'Z') c += 'a' - 'A';
    }
    size_t start = 0;
    while (start < name.size()) {
        size_t end = name.find('\\', start);
        std::string part = name.substr(start, end == std::string::npos ? end : end - start);
        if (part.empty() || part == "." || part == "..") return false;
        if (end == std::string::npos) return true;
        start = end + 1;
    }
    return false;
}
inline bool ParseLayout(const std::vector<BYTE>& bytes, uint64_t fileSize,
    bool extended, std::vector<Entry>& entries) {
    const size_t start = extended ? 8 : 4, trailer = extended ? 20 : 12;
    if (bytes.size() < start) return false;
    DWORD count = Word(bytes.data() + start - 4);
    if (!count || count > 1024 * 1024 || count > (bytes.size() - start) / (5 + trailer)) return false;
    std::vector<Entry> parsed;
    parsed.reserve(count);
    size_t pos = start;
    for (DWORD i = 0; i < count; ++i) {
        if (bytes.size() - pos < 4) return false;
        DWORD length = Word(bytes.data() + pos); pos += 4;
        if (!length || length > 512 || bytes.size() - pos < length + trailer) return false;
        Entry entry{std::string(reinterpret_cast<const char*>(bytes.data() + pos), length), 0, 0};
        pos += length;
        entry.offset = Word(bytes.data() + pos + trailer - 8);
        entry.size = Word(bytes.data() + pos + trailer - 4);
        pos += trailer;
        if (!Normalize(entry.name) || entry.offset < uint64_t(7) + bytes.size() ||
            uint64_t(entry.offset) + entry.size > fileSize) return false;
        parsed.push_back(std::move(entry));
    }
    // PFS can append lookup data after the entries. Keep the complete index for
    // pf8 SHA-1, rather than hashing only the entry records.
    std::sort(parsed.begin(), parsed.end(), [](const Entry& a, const Entry& b) { return a.name < b.name; });
    for (size_t i = 1; i < parsed.size(); ++i) if (parsed[i-1].name == parsed[i].name) return false;
    entries = std::move(parsed);
    return true;
}
inline bool Load(HANDLE file, Archive& archive) {
    // CryptoAPI may load its provider through hooked filesystem APIs. Such
    // internal opens must bypass engine probing to avoid recursive initialization.
    ReadScope reading;
    BYTE header[7]{}; LARGE_INTEGER size{};
    if (!GetFileSizeEx(file, &size) || size.QuadPart < 11 || !Read(file, 0, header, 7)) return false;
    if (header[0] != 'p' || header[1] != 'f' ||
        (header[2] != '2' && header[2] != '6' && header[2] != '8')) return false;
    DWORD indexSize = Word(header + 3);
    if (indexSize < 4 || indexSize > MaxIndex || uint64_t(indexSize) + 7 > uint64_t(size.QuadPart)) return false;
    std::vector<BYTE> bytes(indexSize);
    if (!Read(file, 7, bytes.data(), indexSize)) return false;
    // Shipped pf6/pf8 archives start with count and 12-byte entry metadata.
    // Retain the upstream extended layout only if the entire index validates.
    if (!ParseLayout(bytes, size.QuadPart, false, archive.entries) &&
        !ParseLayout(bytes, size.QuadPart, true, archive.entries)) return false;
    archive.encrypted = header[2] == '8';
    archive.indexBytes = indexSize;
    return !archive.encrypted || Hash(bytes, archive.key);
}
inline bool Resource(HANDLE file, const Archive& archive, const Entry& entry, std::vector<BYTE>& bytes) {
    if (entry.size > MaxResource) return false;
    std::vector<BYTE> result(entry.size);
    if (!Read(file, entry.offset, result.data(), entry.size)) return false;
    if (archive.encrypted) for (size_t i = 0; i < result.size(); ++i) result[i] ^= archive.key[i % 20];
    bytes = std::move(result);
    return true;
}
struct Candidate { std::wstring name; int part; bool executable; };
inline bool CandidateName(std::wstring name, const std::wstring& executableStem, Candidate& candidate) {
    for (wchar_t& c : name) if (c >= L'A' && c <= L'Z') c += L'a' - L'A';
    size_t ext = name.rfind(L".pfs");
    if (ext == std::wstring::npos) return false;
    std::wstring suffix = name.substr(ext + 4);
    int part = -1;
    if (!suffix.empty()) {
        if (suffix.size() != 4 || suffix[0] != L'.') return false;
        part = 0;
        for (size_t i = 1; i < 4; ++i) {
            if (suffix[i] < L'0' || suffix[i] > L'9') return false;
            part = part * 10 + suffix[i] - L'0';
        }
    }
    candidate = {std::move(name), part, false};
    candidate.executable = _wcsnicmp(candidate.name.c_str(), executableStem.c_str(), ext) == 0 && ext == executableStem.size();
    return true;
}
inline bool Before(const Candidate& a, const Candidate& b) {
    // A numbered overlay replaces a base resource; prefer the running executable's
    // family on ties. Unrelated names have a deterministic fallback order.
    if (a.part != b.part) return a.part > b.part;
    if (a.executable != b.executable) return a.executable;
    return a.name > b.name;
}
}
