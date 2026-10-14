#include "injector.h"
#include <filesystem>

auto loader::injector::Injector::inject_module(const std::string& module_path) -> InjectionResult {
	if (!std::filesystem::exists(module_path)) {
		return InjectionResult::FailedDll;
	};

	std::uint64_t alloc_mem = reinterpret_cast<std::uint64_t>(
		VirtualAllocEx(process_handle, nullptr, module_path.size() + 1,
			MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE));
	if (!alloc_mem) {
		return InjectionResult::FailedAlloc;
	};

	if (!WriteProcessMemory(process_handle, reinterpret_cast<LPVOID>(alloc_mem),
		module_path.c_str(), module_path.size() + 1, nullptr)) {
		return InjectionResult::FailedWrite;
	};

	auto h_thread = CreateRemoteThread(process_handle, nullptr, 0,
		reinterpret_cast<LPTHREAD_START_ROUTINE>(load_lib_w_addr),
		reinterpret_cast<LPVOID>(alloc_mem), 0, nullptr);
	if (!h_thread) {
		return InjectionResult::FailedThread;
	};

	Sleep(1000); // wait for the thread to finish
	return InjectionResult::Success;
}