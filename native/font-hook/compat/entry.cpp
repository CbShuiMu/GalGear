#include "framework.h"
#include "hooks/font_hooks.h"
namespace {
DWORD WINAPI Install(void* value) {
    HMODULE pinned = nullptr;
    // Detours are not detachable in upstream; keep all hook code alive until exit.
    if (!GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
        reinterpret_cast<LPCWSTR>(&Install), &pinned)) return 1;
    FontHooks::Install(static_cast<HMODULE>(value));
    return 0;
}
}
BOOL APIENTRY DllMain(HMODULE module, DWORD reason, LPVOID) {
    if (reason == DLL_PROCESS_ATTACH) {
        DisableThreadLibraryCalls(module);
        // Worker starts after loader-lock release; no engine probing inside DllMain.
        HANDLE thread = CreateThread(nullptr, 0, Install, module, 0, nullptr);
        if (!thread) return FALSE;
        CloseHandle(thread);
    } else if (reason == DLL_PROCESS_DETACH) Utils::RequestShutdown();
    return TRUE;
}
