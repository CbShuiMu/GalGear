// Isolated GDI process: reports success only when rasterized glyph height increases.
#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <cstdio>
#include <algorithm>
#include <vector>
#include <cstring>
#include <share.h>
FILE* fixtureOutput = stdout;

struct Glyph {
    GLYPHMETRICS metrics{};
    std::vector<BYTE> bytes;
    bool valid = false;
    bool operator==(const Glyph& other) const {
        return valid && other.valid && memcmp(&metrics, &other.metrics, sizeof(metrics)) == 0 && bytes == other.bytes;
    }
};
Glyph Outline(HDC dc, UINT character, bool ansi = false) {
    MAT2 matrix{}; matrix.eM11.value=1; matrix.eM22.value=1;
    Glyph glyph;
    auto get = ansi ? GetGlyphOutlineA : GetGlyphOutlineW;
    DWORD size = get(dc, character, GGO_GRAY8_BITMAP, &glyph.metrics, 0, nullptr, &matrix);
    if (!size || size == GDI_ERROR) return glyph;
    glyph.bytes.resize(size);
    glyph.valid = get(dc, character, GGO_GRAY8_BITMAP, &glyph.metrics, size, glyph.bytes.data(), &matrix) == size;
    return glyph;
}
int Family(const wchar_t* face) {
    HDC dc = CreateCompatibleDC(nullptr);
    HFONT expected = CreateFontW(-32,0,0,0,400,0,0,0,GB2312_CHARSET,0,0,ANTIALIASED_QUALITY,0,face);
    HGDIOBJ old = SelectObject(dc, expected);
    if (fixtureOutput != stdout) {
        wchar_t selected[LF_FACESIZE]{};
        GetTextFaceW(dc,LF_FACESIZE,selected);
        fprintf(fixtureOutput,"FACE");
        for (wchar_t c : selected) { if (!c) break; fprintf(fixtureOutput," %04x",c); }
        fprintf(fixtureOutput,"\n"); fflush(fixtureOutput);
    }
    const Glyph han = Outline(dc, 0x6c38), kana = Outline(dc, 0x3042);
    HFONT cached = CreateFontW(-32,0,0,0,400,0,0,0,SHIFTJIS_CHARSET,0,0,ANTIALIASED_QUALITY,0,L"MS Mincho");
    SelectObject(dc, cached);
    // The fixture must distinguish font families, not merely see a valid glyph.
    if (!han.valid || !kana.valid || Outline(dc, 0x6c38) == han) return 4;
    fprintf(fixtureOutput,"READY\n"); fflush(fixtureOutput);
    int result = 5;
    for (int i=0;i<400;++i) {
        Sleep(50);
        if (!(Outline(dc, 0x6c38) == han)) continue;
        LOGFONTA logical{}; logical.lfHeight=-32; logical.lfWeight=400;
        logical.lfCharSet=SHIFTJIS_CHARSET; logical.lfQuality=ANTIALIASED_QUALITY;
        strcpy_s(logical.lfFaceName, "MS Mincho");
        HFONT fresh=CreateFontIndirectA(&logical);
        SelectObject(dc, fresh);
        // Shift-JIS bytes for U+3042 must retain their meaning after replacement.
        bool correct=Outline(dc, 0x6c38)==han && Outline(dc, 0x82a0, true)==kana && GetTextCharset(dc)==SHIFTJIS_CHARSET;
        SelectObject(dc, cached); DeleteObject(fresh);
        if (correct) {result=0;break;}
    }
    SelectObject(dc, old); DeleteObject(cached); DeleteObject(expected); DeleteDC(dc);
    return result;
}

int Height() {
    HDC dc = CreateCompatibleDC(nullptr);
    BITMAPINFO info{}; info.bmiHeader.biSize=sizeof(BITMAPINFOHEADER);
    info.bmiHeader.biWidth=400; info.bmiHeader.biHeight=-160;
    info.bmiHeader.biPlanes=1; info.bmiHeader.biBitCount=32;
    void* pixels=nullptr;
    HBITMAP bitmap=CreateDIBSection(dc,&info,DIB_RGB_COLORS,&pixels,nullptr,0);
    if (!dc || !bitmap || !pixels) return -1;
    HGDIOBJ oldBitmap=SelectObject(dc,bitmap);
    PatBlt(dc,0,0,400,160,WHITENESS);
    HFONT font=CreateFontW(-20,0,0,0,400,0,0,0,DEFAULT_CHARSET,0,0,ANTIALIASED_QUALITY,0,L"Arial");
    HGDIOBJ oldFont=SelectObject(dc,font);
    SetTextColor(dc,RGB(0,0,0)); SetBkMode(dc,TRANSPARENT);
    TextOutW(dc,5,5,L"AgMpQ",5); GdiFlush();
    int top=160,bottom=-1;
    auto data=static_cast<unsigned int*>(pixels);
    for(int y=0;y<160;++y) for(int x=0;x<400;++x) if((data[y*400+x]&0xffffff)<0xeeeeee) {top=std::min(top,y);bottom=std::max(bottom,y);}
    SelectObject(dc,oldFont);SelectObject(dc,oldBitmap);DeleteObject(font);DeleteObject(bitmap);DeleteDC(dc);
    return bottom-top+1;
}
int Session() {
    const int baseline = Height();
    if (baseline <= 0) return 2;
    printf("READY %d\n", baseline); fflush(stdout);
    char command[32]{};
    while (fgets(command, sizeof(command), stdin)) {
        if (strcmp(command, "quit\n") == 0) return 0;
        if (strcmp(command, "height\n") != 0) return 9;
        printf("HEIGHT %d\n", Height()); fflush(stdout);
    }
    return 0;
}
// 独立窗口供 LEP x86/x64 截图联调；不接触游戏目录，输出路径由隔离测试提供。
int LocaleWindow(const char* output) {
    FILE* file = _fsopen(output, "w", _SH_DENYNO);
    if (!file) return 7;
    SetProcessDPIAware();
    WNDCLASSW wc{};
    wc.lpfnWndProc = DefWindowProcW;
    wc.hInstance = GetModuleHandleW(nullptr);
    wc.lpszClassName = L"GalGearLocaleScreenshotFixture";
    HBRUSH brush = CreateSolidBrush(RGB(204,102,51));
    wc.hbrBackground = brush;
    if (!RegisterClassW(&wc)) { fclose(file); DeleteObject(brush); return 8; }
    HWND window = CreateWindowExW(WS_EX_TOPMOST, wc.lpszClassName, L"GalGear LEP screenshot fixture",
        WS_OVERLAPPEDWINDOW, 80, 80, 640, 480, nullptr, nullptr, wc.hInstance, nullptr);
    if (!window) { fclose(file); UnregisterClassW(wc.lpszClassName,wc.hInstance); DeleteObject(brush); return 8; }
    ShowWindow(window,SW_SHOW); SetForegroundWindow(window); UpdateWindow(window);
    fprintf(file,"PID %lu ACP %u READY\n",GetCurrentProcessId(),GetACP()); fflush(file);
    ULONGLONG start=GetTickCount64();
    while (IsWindow(window) && GetTickCount64()-start<60000) {
        MSG message{};
        while(PeekMessageW(&message,nullptr,0,0,PM_REMOVE)) {TranslateMessage(&message);DispatchMessageW(&message);}
        Sleep(10);
    }
    if (IsWindow(window)) DestroyWindow(window);
    fclose(file); UnregisterClassW(wc.lpszClassName,wc.hInstance); DeleteObject(brush);
    return 0;
}
int main(int argc, char** argv) {
    if (argc==3 && strcmp(argv[1],"--locale-window")==0) return LocaleWindow(argv[2]);
    if (argc==2 && strcmp(argv[1],"--session")==0) return Session();
    if (argc==4 && strcmp(argv[1],"--locale-family")==0) {
        fixtureOutput = _fsopen(argv[3],"w",_SH_DENYNO);
        if (!fixtureOutput) return 7;
        fprintf(fixtureOutput,"PID %lu ACP %u\n", GetCurrentProcessId(), GetACP()); fflush(fixtureOutput);
        wchar_t face[LF_FACESIZE]{};
        if (!MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,argv[2],-1,face,LF_FACESIZE)) return 6;
        int result=Family(face);
        fprintf(fixtureOutput,"RESULT %d\n",result); fclose(fixtureOutput);
        return result;
    }
    if (argc==3 && strcmp(argv[1],"--family")==0) {
        wchar_t face[LF_FACESIZE]{};
        if (!MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,argv[2],-1,face,LF_FACESIZE)) return 6;
        return Family(face);
    }
    int baseline=Height(); if(baseline<=0) return 2;
    puts("READY");fflush(stdout);
    for(int i=0;i<400;++i) {Sleep(50); if(Height()>=baseline*3/2) { getchar(); return 0; }}
    return 3;
}
