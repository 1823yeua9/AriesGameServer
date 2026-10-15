#pragma once

#include <filesystem>

namespace internal::startup {
    auto prepare(const std::filesystem::path& runtime) -> void;

    auto finish(const std::filesystem::path& runtime) -> void;
} // namespace internal::startup
