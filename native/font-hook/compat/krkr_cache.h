#pragma once
#include <windows.h>
#include <atomic>
#include <cstring>
#include <cstdint>

// KiriKiri caches rendered characters above GDI. Resolve only the verified x86
// compact callback ABI, then run its cache clear on the owning window thread.
// Unknown builds are left untouched; no fixed game addresses or vtable writes.
namespace GalGearKrkr {
using Clear = void (__cdecl*)();
struct Image {
    BYTE* base;
    size_t size;
    bool contains(const void* p, size_t n) const {
        auto value = reinterpret_cast<uintptr_t>(p), start = reinterpret_cast<uintptr_t>(base);
        return value >= start && value - start <= size && n <= size - (value - start);
    }
    bool executable(const void* p) const {
        if (!contains(p, 1)) return false;
        MEMORY_BASIC_INFORMATION info{};
        return VirtualQuery(p, &info, sizeof(info)) && info.State == MEM_COMMIT &&
            !(info.Protect & (PAGE_GUARD | PAGE_NOACCESS)) &&
            (info.Protect & (PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE));
    }
};
inline Clear Resolve(Image image) {
#ifdef _WIN64
    return nullptr;
#else
    const char name[] = ".?AUtTVPClearFontCacheCallback@@";
    Clear found = nullptr;
    for (size_t i = 8; i + sizeof(name) <= image.size; ++i) {
        if (memcmp(image.base + i, name, sizeof(name))) continue;
        auto type = reinterpret_cast<uintptr_t>(image.base + i - 8);
        for (size_t j = 0; j + 20 <= image.size; j += 4) {
            auto col = reinterpret_cast<DWORD*>(image.base + j);
            if (col[0] || col[1] || col[2] || col[3] != type) continue;
            for (size_t k = 0; k + 8 <= image.size; k += 4) {
                auto slot = reinterpret_cast<DWORD*>(image.base + k);
                if (slot[0] != reinterpret_cast<uintptr_t>(col)) continue;
                auto code = reinterpret_cast<BYTE*>(slot[1]);
                // Verified cdecl OnCompact(this, level): level >= 15 tail-calls
                // the no-argument cache clear. Other compiler ABIs are rejected.
                const BYTE prefix[] = {0x55,0x8b,0xec,0x83,0x7d,0x0c,0x0f,0x7c,0x06,0x5d,0xe9};
                if (!image.contains(code, 17) || !image.executable(code) ||
                    memcmp(code, prefix, sizeof(prefix)) || code[15] != 0x5d || code[16] != 0xc3) continue;
                int displacement; memcpy(&displacement, code + 11, 4);
                auto target = code + 15 + displacement;
                if (!image.executable(target)) continue;
                auto clear = reinterpret_cast<Clear>(target);
                if (found && found != clear) return nullptr;
                found = clear;
            }
        }
    }
    return found;
#endif
}
inline std::atomic<Clear> pending{nullptr};
inline Clear Probe(Image image) {
    // Loaded PE images can have discarded/no-access sections. A read fault is
    // an unsupported layout, never a reason to change the game's protections.
    __try { return Resolve(image); }
    __except(EXCEPTION_EXECUTE_HANDLER) { return nullptr; }
}
inline HANDLE completed = nullptr;
inline UINT message = 0;
inline std::atomic<LONG> generation{0};
inline LRESULT CALLBACK Dispatch(int code, WPARAM wp, LPARAM lp) {
    if (code >= 0 && wp == PM_REMOVE) {
        auto msg = reinterpret_cast<MSG*>(lp);
        if (msg->message == message && static_cast<LONG>(msg->wParam) == generation.load()) {
            if (auto clear = pending.exchange(nullptr)) {
                clear();
                SetEvent(completed);
            }
        }
    }
    return CallNextHookEx(nullptr, code, wp, lp);
}
inline void Refresh(LONG version) {
    auto base = reinterpret_cast<BYTE*>(GetModuleHandleW(nullptr));
    auto dos = reinterpret_cast<IMAGE_DOS_HEADER*>(base);
    auto nt = reinterpret_cast<IMAGE_NT_HEADERS*>(base + dos->e_lfanew);
    static Clear clear = Probe({base, nt->OptionalHeader.SizeOfImage});
    if (!clear) return;
    DWORD thread = 0;
    EnumWindows([](HWND window, LPARAM param) -> BOOL {
        DWORD pid = 0; DWORD tid = GetWindowThreadProcessId(window, &pid);
        if (pid == GetCurrentProcessId() && IsWindowVisible(window) && !GetWindow(window, GW_OWNER)) {
            *reinterpret_cast<DWORD*>(param) = tid; return FALSE;
        }
        return TRUE;
    }, reinterpret_cast<LPARAM>(&thread));
    if (!thread) return;
    if (!completed) completed = CreateEventW(nullptr, FALSE, FALSE, nullptr);
    if (!message) message = RegisterWindowMessageW(L"GalGear.Krkr.FontCache.v1");
    if (!completed || !message) return;
    HMODULE self = nullptr;
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        reinterpret_cast<LPCWSTR>(&Dispatch), &self)) return;
    HHOOK hook = SetWindowsHookExW(WH_GETMESSAGE, Dispatch, self, thread);
    if (!hook) return;
    ResetEvent(completed);
    generation.store(version);
    pending.store(clear);
    bool posted = PostThreadMessageW(thread, message, version, 0) != 0;
    DWORD result = posted ? WaitForSingleObject(completed, 3000) : WAIT_FAILED;
    pending.store(nullptr);
    UnhookWindowsHookEx(hook);
    Utils::Trace("[GalGearKrkr] cache-clear version=%ld thread=%lu completed=%d", version, thread, result == WAIT_OBJECT_0);
}
}
