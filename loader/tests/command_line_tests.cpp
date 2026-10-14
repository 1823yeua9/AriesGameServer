#include "../src/command_line.h"

#include <stdexcept>
#include <string>
#include <utility>
#include <vector>

namespace loader::tests {
    static auto parse_arguments(std::vector<std::wstring> arguments) -> CommandLine {
        std::vector<wchar_t*> pointers;

        for (auto& argument : arguments) {
            pointers.push_back(argument.data());
        }

        return parse_command_line(static_cast<int>(pointers.size()), pointers.data());
    }

    static auto check_mode(const CommandLine& options, LaunchMode expected) -> void {
        if (options.mode != expected) {
            throw std::runtime_error("command-line mode changed during extraction");
        }
    }

    auto run_command_line_tests() -> void {
        const auto defaults = parse_arguments({L"loader.exe"});

        check_mode(defaults, LaunchMode::Launch);

        if (!defaults.config_path.empty()) {
            throw std::runtime_error("default settings path was overridden");
        }

        check_mode(parse_arguments({L"loader.exe", L"--help"}), LaunchMode::Help);

        const auto path = std::wstring(L"settings directory 测试/settings.val");
        const auto expected_path = std::filesystem::absolute(path);

        const std::vector<std::pair<std::wstring, LaunchMode>> modes = {
            {L"--check", LaunchMode::Check},
            {L"--install-firewall", LaunchMode::InstallFirewall},
            {L"--server-only", LaunchMode::ServerOnly}};

        for (const auto& [flag, mode] : modes) {
            const auto before = parse_arguments({L"loader.exe", flag, L"--config", path});
            const auto after = parse_arguments({L"loader.exe", L"--config", path, flag});

            check_mode(before, mode);
            check_mode(after, mode);

            if (before.config_path != expected_path || after.config_path != expected_path) {
                throw std::runtime_error("config argument order or Unicode path changed");
            }

            check_mode(parse_arguments({L"loader.exe", flag}), mode);
        }

        const auto launch = parse_arguments({L"loader.exe", L"--config", path});
        const auto create = parse_arguments({L"loader.exe", L"--write-config", path});

        check_mode(launch, LaunchMode::Launch);
        check_mode(create, LaunchMode::Template);

        if (launch.config_path != expected_path || create.config_path != expected_path) {
            throw std::runtime_error("launch/template settings path changed");
        }
    }
} // namespace loader::tests
