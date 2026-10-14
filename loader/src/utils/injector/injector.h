#pragma once
#include <Windows.h>
#include <TlHelp32.h>
#include <cstdint>
#include <string>
#include <stdexcept> 

namespace loader::injector {

	enum class InjectionResult : uint8_t {
		Success = 0,
		FailedAlloc,
		FailedWrite,
		FailedThread,
		FailedDll
	};

	class Injector {
		std::uint32_t pid = -1;
		std::uint64_t load_lib_w_addr = 0;
		HANDLE process_handle = INVALID_HANDLE_VALUE;

	public:

		Injector(std::uint32_t pid) : pid(pid) {
			process_handle = OpenProcess(PROCESS_ALL_ACCESS, FALSE, pid);
			if (process_handle == INVALID_HANDLE_VALUE) {
				throw std::runtime_error("Failed to open process");
			}

			load_lib_w_addr = reinterpret_cast<std::uint64_t>( // resolving addr avoids the annoying shit you have to do normally
				GetProcAddress(GetModuleHandleA("kernel32.dll"), 
					"LoadLibraryW"));
		}

		~Injector() {
			if (process_handle != INVALID_HANDLE_VALUE) {
				CloseHandle(process_handle);
			}
		}

		[[nodiscard]] auto inject_module(const std::string& module_path) -> InjectionResult;
	};
}