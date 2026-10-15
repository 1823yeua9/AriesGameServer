#include "startup.h"
#include "session.h"

#include <Windows.h>
#include <array>
#include <chrono>
#include <cstring>
#include <fstream>
#include <stdexcept>
#include <vector>

namespace internal::startup {
    // Build 13.06. Ported from the verified project scripts:
    // launch_memory_initializer_experiment.ps1 (PrivateObject + DelayedPatch)
    // resolve_init_screen_reconcile.py / test_init_screen_reconcile.ps1.
    // Every address is image-relative or resolved from this process's object graph.
    static constexpr std::uintptr_t manager_table_rva = 0xB26BA30;
    static constexpr std::uintptr_t initializer_rva = 0x6B62B30;
    static constexpr std::uintptr_t alternate_rva = 0x644CF30;
    static constexpr std::uintptr_t flow_table_rva = 0xAF7A990;
    static constexpr std::uintptr_t state_table_rva = 0xAFB6440;
    static constexpr std::uintptr_t reconcile_rva = 0x6C9E0F0;
    static constexpr size_t dispatch_offset = 0x2F0;
    static std::uintptr_t image_base = 0;
    static std::vector<std::uintptr_t> managers;
    static void* dispatch_clone = nullptr;

    template <typename Value> static auto read(std::uintptr_t address) -> Value {
        Value result{};
        SIZE_T received = 0;

        if (!ReadProcessMemory(
                GetCurrentProcess(), reinterpret_cast<void*>(address), &result, sizeof(result), &received) ||
            received != sizeof(result)) {
            throw std::runtime_error("Startup object is unreadable; no unchecked write performed");
        }

        return result;
    }

    static auto log_tail(const std::filesystem::path& runtime) -> std::string {
        std::ifstream file(runtime / "game.log", std::ios::binary);

        if (!file) {
            return {};
        }

        file.seekg(0, std::ios::end);
        const auto length = static_cast<std::streamoff>(file.tellg());
        file.seekg(std::max<std::streamoff>(0, length - 1024 * 1024));

        return {std::istreambuf_iterator<char>(file), {}};
    }

    static auto assert_no_failure(const std::string& text) -> void {
        for (const auto* marker : {"Failed to initialize: VanguardManager",
                                   "VanguardInitFail",
                                   "Marking Platform Faulted",
                                   "Status is now: InitFailure"}) {
            if (text.find(marker) != std::string::npos) {
                throw std::runtime_error("Client reported initialization failure");
            }
        }
    }

    template <size_t Size>
    static auto require_bytes(std::uintptr_t address, const std::array<unsigned char, Size>& expected) -> void {
        if (read<std::array<unsigned char, Size>>(address) != expected) {
            throw std::runtime_error("Pinned startup code bytes differ; refusing this build");
        }
    }

    template <typename Predicate>
    static auto scan(std::uintptr_t needle, size_t max_bytes, Predicate accepts) -> std::vector<std::uintptr_t> {
        const auto deadline = GetTickCount64() + 12000;
        std::vector<std::uintptr_t> found;
        std::vector<std::uintptr_t> buffer(1024 * 1024 / sizeof(std::uintptr_t));
        ULONG_PTR stack_low = 0, stack_high = 0;
        GetCurrentThreadStackLimits(&stack_low, &stack_high);
        const auto scratch_begin = reinterpret_cast<std::uintptr_t>(buffer.data());
        const auto scratch_end = scratch_begin + buffer.size() * sizeof(std::uintptr_t);
        size_t scanned = 0;

        for (std::uintptr_t address = 0x10000; address < 0x0000800000000000;) {
            MEMORY_BASIC_INFORMATION region{};

            if (!VirtualQuery(reinterpret_cast<void*>(address), &region, sizeof(region))) {
                break;
            }

            const auto begin = reinterpret_cast<std::uintptr_t>(region.BaseAddress);
            const auto end = begin + region.RegionSize;

            if (end <= address) {
                throw std::runtime_error("Startup scan did not advance");
            }

            address = end;
            const auto protection = region.Protect & 0xFF;

            if (region.State != MEM_COMMIT || region.Type != MEM_PRIVATE || (region.Protect & PAGE_GUARD) ||
                (protection != PAGE_READWRITE && protection != PAGE_WRITECOPY)) {
                continue;
            }

            for (auto cursor = begin; cursor < end;) {
                const auto count = std::min<size_t>(buffer.size() * sizeof(std::uintptr_t), end - cursor);

                if (scanned + count > max_bytes || GetTickCount64() >= deadline) {
                    throw std::runtime_error("Startup scan budget exceeded; no partial result accepted");
                }

                SIZE_T received = 0;
                scanned += count;

                if (ReadProcessMemory(
                        GetCurrentProcess(), reinterpret_cast<void*>(cursor), buffer.data(), count, &received)) {
                    for (size_t index = 0; index < received / sizeof(std::uintptr_t); ++index) {
                        const auto candidate = cursor + index * sizeof(std::uintptr_t);

                        // Unlike the original out-of-process scanner, this worker has
                        // its own copies of the needle inside the target address space.
                        if ((candidate >= stack_low && candidate < stack_high) ||
                            (candidate >= scratch_begin && candidate < scratch_end)) {
                            continue;
                        }

                        if (buffer[index] == needle && accepts(candidate)) {
                            found.push_back(candidate);

                            if (found.size() > 4) {
                                throw std::runtime_error("Too many startup object candidates");
                            }
                        }
                    }
                }

                cursor += count;
            }
        }

        return found;
    }

    static auto restore() -> void {
        if (!dispatch_clone) {
            return;
        }

        auto* replacement = static_cast<unsigned char*>(dispatch_clone) + 8;
        auto* original = reinterpret_cast<void*>(image_base + manager_table_rva);

        for (const auto address : managers) {
            const auto current = read<void*>(address);

            if (current == replacement) {
                if (InterlockedCompareExchangePointer(
                        reinterpret_cast<void* volatile*>(address), original, replacement) != replacement) {
                    throw std::runtime_error("Startup manager changed during restore");
                }
            }
            else if (current != original) {
                throw std::runtime_error("Startup manager no longer has the expected dispatch table");
            }
        }

        // Retain the 4 KB read-only clone until process teardown. Another thread
        // could have loaded its address before the atomic restore; never free beneath it.
        session::log("Original client dispatch tables restored; services untouched");
    }

    auto prepare(const std::filesystem::path& runtime) -> void {
        if (!std::filesystem::is_regular_file(runtime / "startup.armed") ||
            !std::filesystem::is_regular_file(runtime / "startup.waiting")) {
            throw std::runtime_error("Startup configuration gate is not held");
        }

        const auto log = log_tail(runtime);
        assert_no_failure(log);

        if (log.find("Initializing: VanguardManager") != std::string::npos) {
            throw std::runtime_error("Client initializer already entered; refusing late changes");
        }

        image_base = reinterpret_cast<std::uintptr_t>(GetModuleHandleW(nullptr));
        require_bytes(image_base, std::array<unsigned char, 2>{0x4D, 0x5A});
        require_bytes(image_base + alternate_rva,
                      std::array<unsigned char, 8>{0x48, 0x89, 0x5C, 0x24, 0x08, 0x4C, 0x89, 0x44});
        const auto original = image_base + manager_table_rva;
        auto table = read<std::array<unsigned char, 4096>>(original - 8);
        std::uintptr_t entry = 0;
        std::memcpy(&entry, table.data() + dispatch_offset, sizeof(entry));

        if (entry != image_base + initializer_rva) {
            throw std::runtime_error("Client initializer dispatch differs");
        }

        managers = scan(original, size_t(2) << 30, [](std::uintptr_t) {
            return true;
        });

        if (managers.empty()) {
            throw std::runtime_error("Client manager not found; gate remains held");
        }

        const auto alternate = image_base + alternate_rva;
        std::memcpy(table.data() + dispatch_offset, &alternate, sizeof(alternate));
        dispatch_clone = VirtualAlloc(nullptr, table.size(), MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);

        if (!dispatch_clone) {
            throw std::runtime_error("Cannot allocate private dispatch table");
        }

        std::memcpy(dispatch_clone, table.data(), table.size());
        DWORD previous = 0;

        if (!VirtualProtect(dispatch_clone, table.size(), PAGE_READONLY, &previous)) {
            throw std::runtime_error("Cannot protect private dispatch table");
        }

        try {
            for (const auto address : managers) {
                if (InterlockedCompareExchangePointer(reinterpret_cast<void* volatile*>(address),
                                                      static_cast<unsigned char*>(dispatch_clone) + 8,
                                                      reinterpret_cast<void*>(original)) !=
                    reinterpret_cast<void*>(original)) {
                    throw std::runtime_error("Client manager changed; rolling back private dispatch");
                }
            }
        }
        catch (...) {
            restore();
            throw;
        }

        session::log("Prepared verified private client dispatch; objects=" + std::to_string(managers.size()));
    }

    static auto reconcile_call(std::uintptr_t address, std::uintptr_t manager) -> bool {
        __try {
            reinterpret_cast<void(__fastcall*)(void*)>(address)(reinterpret_cast<void*>(manager));
            return true;
        }
        __except (EXCEPTION_EXECUTE_HANDLER) {
            return false;
        }
    }

    auto finish(const std::filesystem::path& runtime) -> void {
        auto deadline = GetTickCount64() + 60000;

        try {
            for (;;) {
                const auto log = log_tail(runtime);
                assert_no_failure(log);

                if (log.find("Initialized: VanguardManager") != std::string::npos) {
                    break;
                }

                if (GetTickCount64() >= deadline) {
                    throw std::runtime_error("Client initialization completion timed out");
                }

                Sleep(50);
            }
        }
        catch (...) {
            restore();
            throw;
        }

        restore();
        deadline = GetTickCount64() + 90000;

        for (;;) {
            const auto log = log_tail(runtime);
            assert_no_failure(log);

            if (log.find("MainMenuV2") != std::string::npos) {
                session::log("Main menu reached without reconciliation");
                return;
            }

            if (log.find("Platform initialization succeeded. Initialized 121 systems out of 121") !=
                std::string::npos) {
                break;
            }

            if (GetTickCount64() >= deadline) {
                throw std::runtime_error("Platform initialization did not complete; no reconciliation attempted");
            }

            Sleep(100);
        }

        require_bytes(
            image_base + reconcile_rva,
            std::array<unsigned char, 16>{
                0x48, 0x89, 0x5C, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7C, 0x24, 0x20, 0x55});
        const auto candidates = scan(image_base + flow_table_rva, size_t(4) << 30, [](std::uintptr_t manager) {
            try {
                const auto state = read<std::uintptr_t>(manager + 0xB0);

                if (!state || state % 8 || read<std::uintptr_t>(state) != image_base + state_table_rva) {
                    return false;
                }

                const auto events = read<std::uintptr_t>(state + 0x58);
                return events && !(events % 8) && read<unsigned char>(events + 0x62) <= 1;
            }
            catch (...) {
                return false;
            }
        });

        if (candidates.size() != 1) {
            throw std::runtime_error("Expected one verified initialization GameFlow graph");
        }

        const auto manager = candidates.front();
        const auto state = read<std::uintptr_t>(manager + 0xB0);
        const auto events = read<std::uintptr_t>(state + 0x58);
        const auto flag = events + 0x62;

        if (read<std::uintptr_t>(manager) != image_base + flow_table_rva ||
            read<std::uintptr_t>(manager + 0xB0) != state ||
            read<std::uintptr_t>(state) != image_base + state_table_rva ||
            read<std::uintptr_t>(state + 0x58) != events || read<unsigned char>(flag) != 0) {
            throw std::runtime_error("GameFlow graph changed before reconciliation");
        }

        SIZE_T written = 0;
        const unsigned char complete = 1;

        if (!WriteProcessMemory(GetCurrentProcess(), reinterpret_cast<void*>(flag), &complete, 1, &written) ||
            written != 1 || read<unsigned char>(flag) != 1) {
            throw std::runtime_error("GameFlow completion flag write failed");
        }

        if (!reconcile_call(image_base + reconcile_rva, manager)) {
            throw std::runtime_error("Verified GameFlow reconciliation raised an exception");
        }

        session::log("121/121 initialized; verified GameFlow reconciliation completed");
    }
} // namespace internal::startup
