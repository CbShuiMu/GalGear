#pragma once
#include <windows.h>
#include <tlhelp32.h>
#include <vector>
// Late loading differs from upstream's startup proxy: enlist running threads before commit.
class GalGearThreadEnlistment {
    std::vector<HANDLE> handles;
public:
    bool Enlist() {
        HANDLE snapshot=CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD,0);
        if(snapshot==INVALID_HANDLE_VALUE) return false;
        THREADENTRY32 entry{};entry.dwSize=sizeof(entry);
        bool ok=Thread32First(snapshot,&entry)!=FALSE;
        if(ok) do {
            if(entry.th32OwnerProcessID!=GetCurrentProcessId() || entry.th32ThreadID==GetCurrentThreadId()) continue;
            HANDLE thread=OpenThread(THREAD_SUSPEND_RESUME|THREAD_GET_CONTEXT|THREAD_SET_CONTEXT|THREAD_QUERY_INFORMATION,FALSE,entry.th32ThreadID);
            if(!thread) {if(GetLastError()==ERROR_INVALID_PARAMETER) continue;ok=false;break;}
            handles.push_back(thread);
        } while(Thread32Next(snapshot,&entry));
        CloseHandle(snapshot);
        // Collect handles before Detours suspends any thread: avoid vector growth
        // while a suspended game thread could own a runtime allocator lock.
        if(ok) for(HANDLE thread:handles) {
            if(DetourUpdateThread(thread)!=NO_ERROR){ok=false;break;}
        }
        return ok;
    }
    ~GalGearThreadEnlistment(){for(HANDLE handle:handles) CloseHandle(handle);}
};
