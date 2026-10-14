#include <Windows.h>
#include <string>

BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, LPVOID) {
    if (reason == DLL_PROCESS_ATTACH) {
        DisableThreadLibraryCalls(instance);

        const auto name = L"Local\\Aries.Test.DllLoaded." + std::to_wstring(GetCurrentProcessId());
        const auto event = OpenEventW(EVENT_MODIFY_STATE, FALSE, name.c_str());

        if (event) {
            SetEvent(event);
            CloseHandle(event);
        }
    }

    return TRUE;
}
