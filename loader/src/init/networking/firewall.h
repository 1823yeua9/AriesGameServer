#pragma once

#include <cstdint>
#include <string>

namespace loader::init::networking {
    enum class FirewallResult : std::uint8_t { Success = 0, FailedComInit, FailedPolicy, FailedRule, FailedResetTask };

    // Verify containment by default; installation is an explicit launcher mode.
    // Setup also installs boot-time disabling; explicit setup re-enables rules.
    // Remote hosts are intentionally unsupported by the offline launch profile.
    [[nodiscard]] auto contain_loopback(const std::wstring& exe_path, bool install) -> FirewallResult;

    [[nodiscard]] auto allow_loopback(const std::wstring& exe_path) -> FirewallResult;

    [[nodiscard]] auto allow_host(const std::wstring& exe_path, const std::wstring& host_ip) -> FirewallResult;
} // namespace loader::init::networking
