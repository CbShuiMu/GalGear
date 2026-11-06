// One font request per helper. Elevation covers loading and WM_COPYDATA.
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <tlhelp32.h>
#include <string>
#include <cstdint>
#include <vector>
#include "protocol.h"

namespace {
// Only AccessDenied permits one UAC retry in the Rust client.
enum Result { Success=0, Invalid=2, AccessDenied=3, Architecture=4, Kernel=5,
    Memory=6, Thread=7, LoadTimeout=8, EndpointTimeout=9, IpcFailure=10,
    Rejected=11, Busy=12, TargetChanged=13, ProcessFailure=14, MissingDll=15, Incompatible=16, FontNotFound=17, PreparationFailed=18 };
struct Handle {
    HANDLE value;
    explicit Handle(HANDLE handle): value(handle) {}
    ~Handle() { if(value && value!=INVALID_HANDLE_VALUE) CloseHandle(value); }
    Handle(const Handle&)=delete;
    Handle& operator=(const Handle&)=delete;
};
bool Number(const wchar_t* text, unsigned base, uint64_t limit, uint64_t& result) {
    if(!*text) return false;
    result=0;
    for(;*text;++text) {
        unsigned digit = *text>=L'0' && *text<=L'9' ? *text-L'0' :
            *text>=L'a' && *text<=L'f' ? *text-L'a'+10 : 99;
        if(digit>=base || result>(limit-digit)/base) return false;
        result=result*base+digit;
    }
    return true;
}
bool Decode(const wchar_t* text, GalGearFont::Packet& packet) {
    if(wcslen(text)!=sizeof(packet)*2) return false;
    auto bytes=reinterpret_cast<unsigned char*>(&packet);
    for(size_t i=0;i<sizeof(packet);++i) {
        wchar_t pair[]{text[i*2],text[i*2+1],0}; uint64_t value=0;
        if(!Number(pair,16,255,value)) return false;
        bytes[i]=static_cast<unsigned char>(value);
    }
    return GalGearFont::Valid(packet);
}
HWND Endpoint(DWORD pid) {
    wchar_t title[32]{}; swprintf_s(title,L"%lu",pid);
    HWND window=FindWindowExW(HWND_MESSAGE,nullptr,GalGearFont::ClassName,title);
    DWORD owner=0; GetWindowThreadProcessId(window,&owner);
    return owner==pid ? window : nullptr;
}
bool SameProcess(HANDLE process, uint64_t expected) {
    FILETIME created{},exit{},kernel{},user{};
    return GetProcessTimes(process,&created,&exit,&kernel,&user) &&
        ((static_cast<uint64_t>(created.dwHighDateTime)<<32)|created.dwLowDateTime)==expected;
}
int Load(HANDLE process, DWORD pid) {
    // Never accept an arbitrary DLL/path from a lower-privilege caller.
    std::wstring path(32768,L'\0');
    DWORD length=GetModuleFileNameW(nullptr,path.data(),static_cast<DWORD>(path.size()));
    if(!length || length>=path.size()) return MissingDll;
    path.resize(length);
    auto slash=path.find_last_of(L"\\/");
    if(slash==std::wstring::npos) return MissingDll;
    path.resize(slash+1); path+=L"GalGearFontHook.dll";
    DWORD attributes=GetFileAttributesW(path.c_str());
    if(attributes==INVALID_FILE_ATTRIBUTES || (attributes&FILE_ATTRIBUTE_DIRECTORY)) return MissingDll;
    // Inspect exports without executing DllMain: an official/unadapted DLL must never open its picker.
    HMODULE image = LoadLibraryExW(path.c_str(), nullptr, DONT_RESOLVE_DLL_REFERENCES);
    if (!image) return Incompatible;
    bool compatible = GetProcAddress(image, "GalGearFontBridgeV2") || GetProcAddress(image, "_GalGearFontBridgeV2");
    FreeLibrary(image);
    if (!compatible) return Incompatible;
    Handle snapshot(CreateToolhelp32Snapshot(TH32CS_SNAPMODULE|TH32CS_SNAPMODULE32,pid));
    MODULEENTRY32W entry{}; entry.dwSize=sizeof(entry); uintptr_t base=0;
    if(snapshot.value!=INVALID_HANDLE_VALUE && Module32FirstW(snapshot.value,&entry)) do {
        if(_wcsicmp(entry.szModule,L"kernel32.dll")==0) base=reinterpret_cast<uintptr_t>(entry.modBaseAddr);
    } while(Module32NextW(snapshot.value,&entry));
    HMODULE kernel=GetModuleHandleW(L"kernel32.dll");
    auto load=GetProcAddress(kernel,"LoadLibraryW");
    HMODULE owner=nullptr;
    if(!base || !load || !GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS|GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        reinterpret_cast<LPCWSTR>(load),&owner) || owner!=kernel) return Kernel;
    const size_t bytes=(path.size()+1)*sizeof(wchar_t);
    void* remote=VirtualAllocEx(process,nullptr,bytes,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);
    SIZE_T written=0;
    if(!remote || !WriteProcessMemory(process,remote,path.c_str(),bytes,&written) || written!=bytes) {
        DWORD error=GetLastError();
        if(remote) VirtualFreeEx(process,remote,0,MEM_RELEASE);
        return error==ERROR_ACCESS_DENIED ? AccessDenied : Memory;
    }
    auto start=reinterpret_cast<LPTHREAD_START_ROUTINE>(base+reinterpret_cast<uintptr_t>(load)-reinterpret_cast<uintptr_t>(kernel));
    Handle thread(CreateRemoteThread(process,nullptr,0,start,remote,0,nullptr));
    if(!thread.value) {
        DWORD error=GetLastError(); VirtualFreeEx(process,remote,0,MEM_RELEASE);
        return error==ERROR_ACCESS_DENIED ? AccessDenied : Thread;
    }
    DWORD wait=WaitForSingleObject(thread.value,10000);
    // A pending remote call may still read the path. Never free it on timeout.
    if(wait==WAIT_OBJECT_0) VirtualFreeEx(process,remote,0,MEM_RELEASE);
    return wait==WAIT_OBJECT_0 ? Success : LoadTimeout;
}
int Apply(HANDLE process, DWORD pid, const GalGearFont::Packet& packet, uintptr_t packetId) {
    USHORT machine=0,native=0;
    if(!IsWow64Process2(process,&machine,&native)) return Architecture;
#ifdef _WIN64
    const USHORT own=IMAGE_FILE_MACHINE_AMD64;
#else
    const USHORT own=IMAGE_FILE_MACHINE_I386;
#endif
    if((machine?machine:native)!=own) return Architecture;
    HWND endpoint=Endpoint(static_cast<DWORD>(pid));
    if(!endpoint) {
        int result=Load(process,static_cast<DWORD>(pid));
        if(result!=Success) return result;
        for(unsigned i=0;i<200 && !endpoint;++i) {
            if(WaitForSingleObject(process,50)!=WAIT_TIMEOUT) return TargetChanged;
            endpoint=Endpoint(static_cast<DWORD>(pid));
        }
        if(!endpoint) return EndpointTimeout;
    }
    DWORD_PTR capabilities = 0;
    if (!SendMessageTimeoutW(endpoint, GalGearFont::QueryMessage, 0, 0,
        SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT, 2000, &capabilities) ||
        (capabilities >> 16) != 2 || (capabilities & 0xff) != 0xff) return Incompatible;
    COPYDATASTRUCT data{packetId,sizeof(packet),const_cast<GalGearFont::Packet*>(&packet)};
    DWORD_PTR response=0; SetLastError(ERROR_SUCCESS);
    if(!SendMessageTimeoutW(endpoint,WM_COPYDATA,0,reinterpret_cast<LPARAM>(&data),SMTO_ABORTIFHUNG|SMTO_BLOCK|SMTO_ERRORONEXIT,10000,&response)) {
        return GetLastError()==ERROR_ACCESS_DENIED ? AccessDenied : IpcFailure;
    }
    return response==1 ? Success : response==2 ? Busy : response==3 ? FontNotFound : response==4 ? PreparationFailed : Rejected;
}
}
int wmain(int argc, wchar_t** argv) {
    GalGearFont::Packet packet{};
    uint64_t pid=0,creation=0;
    if(argc!=4 || !Number(argv[1],10,MAXDWORD,pid) || !pid ||
        !Number(argv[2],16,UINT64_MAX,creation) || !creation || !Decode(argv[3],packet)) return Invalid;
    Handle process(OpenProcess(PROCESS_CREATE_THREAD|PROCESS_QUERY_INFORMATION|PROCESS_VM_OPERATION|PROCESS_VM_WRITE|PROCESS_VM_READ|SYNCHRONIZE,FALSE,static_cast<DWORD>(pid)));
    if(!process.value) return GetLastError()==ERROR_ACCESS_DENIED ? AccessDenied : ProcessFailure;
    // Revalidate after UAC: the game might have exited while consent was pending.
    if(!SameProcess(process.value,creation) || WaitForSingleObject(process.value,0)!=WAIT_TIMEOUT) return TargetChanged;
    return Apply(process.value,static_cast<DWORD>(pid),packet,GalGearFont::PacketId);
}
