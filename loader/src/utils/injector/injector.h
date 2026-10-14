#pragma once
#include <Windows.h>
#include <TlHelp32.h>
#include <cstdint>
#include <string>
#include <stdexcept> 
#include <filesystem>

namespace loader::injector {

	enum class InjectionResult : uint8_t {
		Success = 0,
		FailedAlloc,
		FailedWrite,
		FailedThread,
		TimedOut,
		FailedDll
	};

	class Injector {
		std::uint32_t pid = -1;
		std::uint64_t load_lib_w_addr = 0;
		HANDLE process_handle = INVALID_HANDLE_VALUE;
		DWORD thread_exit_code = 0;

	public:

		Injector(std::uint32_t process_id);
		Injector(const Injector&) = delete;
		auto operator=(const Injector&) -> Injector& = delete;

		~Injector() {
			if (process_handle && process_handle != INVALID_HANDLE_VALUE) {
				CloseHandle(process_handle);
			}
		}

		[[nodiscard]] auto inject_module(const std::filesystem::path& module_path) -> InjectionResult;
		[[nodiscard]] auto last_thread_exit_code() const -> DWORD { return thread_exit_code; }
	};
}
