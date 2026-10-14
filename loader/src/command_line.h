#pragma once

#include <filesystem>

namespace loader {
    enum class LaunchMode { Launch, Check, InstallFirewall, Template, ServerOnly, Help };

    struct CommandLine {
        std::filesystem::path config_path;
        LaunchMode mode = LaunchMode::Launch;
    };

    auto parse_command_line(int argc, wchar_t** argv) -> CommandLine;

    auto print_help() -> void;
} // namespace loader
