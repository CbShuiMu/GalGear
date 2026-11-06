// Included inside FontPicker. The upstream visible UI is replaced by a message
// window; table preparation is delegated to the runtime compatibility helper.
static void BridgeApply(const GalGearFont::Packet& p, const std::wstring& face, const std::string& ansi) {
    // All allocations and font-table work completed before changing live Config.
    wcsncpy_s(Config::ForcedFontNameW, face.c_str(), _TRUNCATE);
    strncpy_s(Config::ForcedFontNameA, ansi.c_str(), _TRUNCATE);
    wcsncpy_s(Config::SourceFontNameW, p.face, _TRUNCATE);
    Config::EnableFaceNameReplace = p.face[0] != 0;
    Config::EnableCharsetReplace = false;
    Config::FontHeightScale = p.height / 100.0f;
    Config::FontWidthScale = p.width / 100.0f;
    Config::EnableFontHeightScale = p.height != 100;
    Config::EnableFontWidthScale = p.width != 100;
    Config::FontCharSpacing = p.spacing;
    Config::EnableFontCharSpacing = p.spacing != 0;
    Config::FontLineSpacing = p.line;
    Config::EnableFontLineSpacing = p.line != 0;
    Config::FontWeight = p.weight;
    Config::EnableFontWeight = p.weight != 400;
    Config::EnableFontVerticalMetrics = p.vertical != 0;
    Config::FontAscentPermille = p.ascent;
    Config::FontDescentPermille = p.descent;
    Config::EnableCodepageSpoof = p.spoof != 0;
    Config::SpoofFromCharset = p.from;
    Config::SpoofToCharset = p.to;
    Config::EnableCodepageRuntimeReplace = p.spoof && p.from != p.to;
    Config::EnableTextSubstitution = p.mapping != 0;
    Config::TextSubstitutionMode = p.mapping ? p.mapping - 1 : 0;
    Config::TextSubstitutionCodepage = p.codepage;
    RefreshFontHookEnabled();
}
static LRESULT CALLBACK BridgeProcedure(HWND hwnd, UINT message, WPARAM wp, LPARAM lp) {
    if (message == GalGearFont::QueryMessage) return GalGearFont::Capabilities;
    if (message == WM_COPYDATA) {
        const auto* data = reinterpret_cast<const COPYDATASTRUCT*>(lp);
        if (!data || data->dwData != GalGearFont::PacketId || data->cbData != sizeof(GalGearFont::Packet) || !data->lpData) return 0;
        GalGearFont::Packet p{}; memcpy(&p, data->lpData, sizeof(p));
        if (!GalGearFont::Valid(p)) return 0;
        if (InterlockedCompareExchange(&g_pickerNotifyActive, 0, 0)) return 2;
        std::wstring face;
        std::string ansi;
        bool found = false;
        try {
            if (!GalGearFace::Resolve(p, face, ansi, found)) return found ? 4 : 3;
        } catch (const std::bad_alloc&) {
            return 4; // No partially published configuration or exception into the game.
        }
        BridgeApply(p, face, ansi);
        ForceGameRedraw();
        // ACK means accepted and refresh requested, not proof of engine repaint.
        // GalGear persists per-game profiles; do not write shared FontHook.ini.
        return 1;
    }
    if (message == WM_CLOSE) { DestroyWindow(hwnd); PostQuitMessage(0); return 0; }
    return DefWindowProcW(hwnd, message, wp, lp);
}
void Init(HMODULE module) {
    g_hModule = module;
    InterlockedExchange(&g_messageThreadId, GetCurrentThreadId());
    WNDCLASSW cls{}; cls.hInstance = module; cls.lpfnWndProc = BridgeProcedure; cls.lpszClassName = GalGearFont::ClassName;
    if (!RegisterClassW(&cls) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) return;
    wchar_t title[32]{}; swprintf_s(title, L"%lu", GetCurrentProcessId());
    // Message-only window: no visible picker, animation, taskbar entry or hotkey.
    g_hWnd = CreateWindowExW(0, GalGearFont::ClassName, title, 0, 0,0,0,0, HWND_MESSAGE, nullptr, module, nullptr);
    if (!g_hWnd) return;
    MSG msg{};
    while (!Utils::IsShuttingDown() && GetMessageW(&msg, nullptr, 0, 0) > 0) {
        TranslateMessage(&msg); DispatchMessageW(&msg);
    }
    if (IsWindow(g_hWnd)) DestroyWindow(g_hWnd);
    g_hWnd = nullptr;
    InterlockedExchange(&g_messageThreadId, 0);
}
void Shutdown() {
    DWORD thread = InterlockedCompareExchange(&g_messageThreadId, 0, 0);
    if (thread) PostThreadMessageW(thread, WM_QUIT, 0, 0);
}
void Toggle() {}
bool IsVisible() { return false; }
