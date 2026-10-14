#include "command_line.h"

#include <iostream>
#include <stdexcept>
#include <string>

namespace loader {
    auto parse_command_line(int argc, wchar_t** argv) -> CommandLine {
        CommandLine options;

        auto mode_seen = false;
        auto config_seen = false;

        const auto select_mode = [&](LaunchMode mode) {
            if (mode_seen) {
                throw std::runtime_error("conflicting or repeated command-line modes; use --help");
            }

            mode_seen = true;
            options.mode = mode;
        };

        for (int index = 1; index < argc; ++index) {
            const std::wstring argument = argv[index];

            if (argument == L"--help") {
                if (argc != 2) {
                    throw std::runtime_error("--help must be used alone");
                }

                options.mode = LaunchMode::Help;

                return options;
            }

            if ((argument == L"--config" || argument == L"--write-config") && index + 1 < argc) {
                if (config_seen) {
                    throw std::runtime_error("only one settings path may be supplied");
                }

                const std::wstring value = argv[++index];

                if (value.empty() || value.starts_with(L"--")) {
                    throw std::runtime_error("missing settings path; use --help");
                }

                config_seen = true;
                options.config_path = std::filesystem::absolute(value);

                if (argument == L"--write-config") {
                    select_mode(LaunchMode::Template);
                }
            }
            else if (argument == L"--check") {
                select_mode(LaunchMode::Check);
            }
            else if (argument == L"--install-firewall") {
                select_mode(LaunchMode::InstallFirewall);
            }
            else if (argument == L"--server-only") {
                select_mode(LaunchMode::ServerOnly);
            }
            else {
                throw std::runtime_error("invalid or incomplete command line; use --help");
            }
        }

        return options;
    }

    auto print_help() -> void {
        std::cout << "Aries loader --config <settings.val> [--check | --install-firewall | --server-only]\n"
                  << "Aries loader --write-config <settings.val>\n"
                  << "Launch owns only its children. R enters Range, Q/Ctrl+C closes the launch.\n";
    }
} // namespace loader
