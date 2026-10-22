#pragma once

namespace loader::init::networking {
    // Caller initializes COM. Install before enabling any persistent block rule.
    [[nodiscard]] auto ensure_firewall_reset(bool install) -> bool;
} // namespace loader::init::networking
