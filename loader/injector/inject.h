#pragma once
#include <Windows.h>

constexpr const char* process_name = "VALORANT-Win64-Shipping.exe";

class Injector final {
	HANDLE h_proc; 

public:
	Injector() = default;
	~Injector() = default;

	auto inject(const char* dll_path) -> bool;
};