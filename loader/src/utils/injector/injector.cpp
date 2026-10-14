#include "injector.h"
#include <Psapi.h>

namespace loader::injector {
static auto module_base(std::uint32_t pid, const std::filesystem::path& path, bool full_path) -> std::uint64_t {
    HANDLE snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
    if (snapshot == INVALID_HANDLE_VALUE) return 0;
    MODULEENTRY32W entry{};
    entry.dwSize = sizeof(entry);
    std::uint64_t result = 0;
    HANDLE process = full_path ? OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, FALSE, pid) : nullptr;
    if (Module32FirstW(snapshot, &entry)) do {
        wchar_t unicode_path[32768]{};
        // Toolhelp's szExePath loses characters outside the system code page
        // on this Windows host, even through Module32FirstW. Query by HMODULE.
        if (full_path && (!process || !K32GetModuleFileNameExW(process, entry.hModule, unicode_path, 32768))) continue;
        const auto candidate = full_path ? std::filesystem::path(unicode_path) : std::filesystem::path(entry.szModule);
        std::error_code error;
        const auto matches = full_path ? std::filesystem::equivalent(candidate, path, error) : _wcsicmp(candidate.c_str(), path.c_str()) == 0;
        if (matches && !error) {
            result = reinterpret_cast<std::uint64_t>(entry.modBaseAddr);
            break;
        }
    } while (Module32NextW(snapshot, &entry));
    if (process) CloseHandle(process);
    CloseHandle(snapshot);
    return result;
}

Injector::Injector(std::uint32_t process_id) : pid(process_id) {
    static_assert(sizeof(void*) == 8, "the loader must be built for x64");
    process_handle = OpenProcess(PROCESS_CREATE_THREAD | PROCESS_QUERY_INFORMATION | PROCESS_VM_OPERATION | PROCESS_VM_WRITE | PROCESS_VM_READ | SYNCHRONIZE, FALSE, pid);
    if (!process_handle) throw std::runtime_error("cannot open target process");
    const auto entry = GetProcAddress(GetModuleHandleW(L"kernel32.dll"), "LoadLibraryW");
    HMODULE owner = nullptr;
    // Resolve the actual owner, including forwarded exports. Only the offset
    // is carried across processes, never this run's local virtual address.
    if (entry && GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, reinterpret_cast<LPCWSTR>(entry), &owner)) {
        wchar_t name[32768]{};
        if (GetModuleFileNameW(owner, name, 32768)) {
            const auto remote_base = module_base(pid, std::filesystem::path(name).filename(), false);
            if (remote_base) load_lib_w_addr = remote_base + reinterpret_cast<std::uint64_t>(entry) - reinterpret_cast<std::uint64_t>(owner);
        }
    }
    if (!load_lib_w_addr) {
        CloseHandle(process_handle);
        process_handle = nullptr;
        throw std::runtime_error("cannot resolve target LoadLibraryW");
    }
}

auto Injector::inject_module(const std::filesystem::path& module_path) -> InjectionResult {
    std::error_code error;
    const auto absolute = std::filesystem::canonical(module_path, error);
    if (error || !std::filesystem::is_regular_file(absolute)) return InjectionResult::FailedDll;
    if (module_base(pid, absolute, true)) return InjectionResult::Success;
    const auto wide_path = absolute.wstring();
    const auto byte_count = (wide_path.size() + 1) * sizeof(wchar_t);
    auto* remote = VirtualAllocEx(process_handle, nullptr, byte_count, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if (!remote) return InjectionResult::FailedAlloc;
    SIZE_T written = 0;
    if (!WriteProcessMemory(process_handle, remote, wide_path.c_str(), byte_count, &written) || written != byte_count) {
        VirtualFreeEx(process_handle, remote, 0, MEM_RELEASE);
        return InjectionResult::FailedWrite;
    }
    HANDLE thread = CreateRemoteThread(process_handle, nullptr, 0, reinterpret_cast<LPTHREAD_START_ROUTINE>(load_lib_w_addr), remote, 0, nullptr);
    if (!thread) {
        VirtualFreeEx(process_handle, remote, 0, MEM_RELEASE);
        return InjectionResult::FailedThread;
    }
    const auto wait_result = WaitForSingleObject(thread, 15000);
    GetExitCodeThread(thread, &thread_exit_code);
    CloseHandle(thread);
    // Timeout: the thread may still use this buffer. The owning launch job
    // terminates its child on failure; never free memory beneath the thread.
    if (wait_result != WAIT_OBJECT_0) return InjectionResult::TimedOut;
    VirtualFreeEx(process_handle, remote, 0, MEM_RELEASE);
    return module_base(pid, absolute, true) ? InjectionResult::Success : InjectionResult::FailedDll;
}
}
