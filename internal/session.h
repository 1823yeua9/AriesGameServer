#pragma once

#include <filesystem>
#include <string>

namespace internal::session {
    auto initialize() -> void;

    auto runtime_directory() -> const std::filesystem::path&;

    auto signal_ready() -> void;

    auto log(const std::string& message) -> void;
} // namespace internal::session
