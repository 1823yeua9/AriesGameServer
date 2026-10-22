#include <WinSock2.h>
#include <WS2tcpip.h>
#include <Windows.h>
#include <netfw.h>
#include <wrl/client.h>
#include <algorithm>
#include <array>
#include <filesystem>
#include <sstream>
#include <vector>
#include "firewall.h"
#include "firewall_reset.h"

#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "oleaut32.lib")
#pragma comment(lib, "ws2_32.lib")

namespace loader::init::networking {
    using Microsoft::WRL::ComPtr;

    class Bstr {
    public:
        BSTR value = nullptr;

        explicit Bstr(const std::wstring& text = {}) : value(SysAllocString(text.c_str())) {}

        ~Bstr() {
            SysFreeString(value);
        }

        auto out() -> BSTR* {
            SysFreeString(value);
            value = nullptr;

            return &value;
        }

        Bstr(const Bstr&) = delete;
        auto operator=(const Bstr&) -> Bstr& = delete;
    };

    using Address = std::array<unsigned char, 16>;
    using AddressRange = std::pair<Address, Address>;

    // Windows may return CIDR or shortened IPv6 for a range we installed. Compare
    // the address sets, not localized netsh output or presentation strings.
    static auto ranges(std::wstring text) -> std::vector<AddressRange> {
        text.erase(std::remove_if(text.begin(), text.end(), iswspace), text.end());

        std::vector<AddressRange> result;
        std::wstringstream stream(text);
        std::wstring part;

        while (std::getline(stream, part, L',')) {
            const int family = part.find(L':') == std::wstring::npos ? AF_INET : AF_INET6;
            const auto width = family == AF_INET ? 32u : 128u;
            const auto slash = part.find(L'/');
            const auto dash = part.find(L'-');
            Address first{}, last{};
            const auto start = part.substr(0, (std::min)(slash, dash));

            if (InetPtonW(family, start.c_str(), first.data()) != 1) {
                throw std::runtime_error("invalid firewall address");
            }

            last = first;

            if (dash != std::wstring::npos) {
                if (InetPtonW(family, part.substr(dash + 1).c_str(), last.data()) != 1) {
                    throw std::runtime_error("invalid firewall range");
                }
            }
            else if (slash != std::wstring::npos) {
                const auto prefix = std::stoul(part.substr(slash + 1));

                if (prefix > width) {
                    throw std::runtime_error("invalid firewall prefix");
                }

                for (auto bit = prefix; bit < width; ++bit) {
                    first[bit / 8] &= static_cast<unsigned char>(~(0x80 >> (bit % 8)));
                    last[bit / 8] |= static_cast<unsigned char>(0x80 >> (bit % 8));
                }
            }

            result.emplace_back(first, last);
        }

        std::sort(result.begin(), result.end());

        return result;
    }

    auto contain_loopback(const std::wstring& exe_path, bool install) -> FirewallResult {
        const auto initialized = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

        if (FAILED(initialized) && initialized != RPC_E_CHANGED_MODE) {
            return FirewallResult::FailedComInit;
        }

        struct ComScope {
            bool owned;

            ~ComScope() {
                if (owned) {
                    CoUninitialize();
                }
            }
        } scope{SUCCEEDED(initialized)};

        try {
            const auto canonical = std::filesystem::canonical(exe_path).make_preferred().wstring();
            std::uint64_t path_hash = 14695981039346656037ull;

            for (wchar_t character : canonical) {
                path_hash ^= towlower(character);
                path_hash *= 1099511628211ull;
            }

            ComPtr<INetFwPolicy2> policy;

            if (FAILED(
                    CoCreateInstance(__uuidof(NetFwPolicy2), nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&policy)))) {
                return FirewallResult::FailedPolicy;
            }

            NET_FW_MODIFY_STATE modify_state{};

            if (FAILED(policy->get_LocalPolicyModifyState(&modify_state)) || modify_state != NET_FW_MODIFY_STATE_OK) {
                return FirewallResult::FailedPolicy;
            }

            for (auto profile : {NET_FW_PROFILE2_DOMAIN, NET_FW_PROFILE2_PRIVATE, NET_FW_PROFILE2_PUBLIC}) {
                VARIANT_BOOL enabled = VARIANT_FALSE;

                if (FAILED(policy->get_FirewallEnabled(profile, &enabled)) || enabled != VARIANT_TRUE) {
                    return FirewallResult::FailedPolicy;
                }
            }

            // Arm reboot cleanup BEFORE adding or re-enabling any block rule.
            // Verification must fail if that safety net has been removed.
            if (!ensure_firewall_reset(install)) {
                return FirewallResult::FailedResetTask;
            }

            ComPtr<INetFwRules> rules;

            if (FAILED(policy->get_Rules(&rules))) {
                return FirewallResult::FailedPolicy;
            }

            const std::array<std::wstring, 2> remotes = {L"0.0.0.0-127.0.0.0,127.0.0.2-255.255.255.255",
                                                         L"::/128,::2-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"};

            for (std::size_t index = 0; index < remotes.size(); ++index) {
                Bstr name(L"Aries offline " + std::to_wstring(path_hash) + L" block " + std::to_wstring(index));
                Bstr application(canonical), remote(remotes[index]), group(L"Aries offline containment");
                ComPtr<INetFwRule> rule;

                if (FAILED(rules->Item(name.value, &rule))) {
                    if (!install) {
                        return FirewallResult::FailedRule;
                    }

                    if (FAILED(CoCreateInstance(
                            __uuidof(NetFwRule), nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&rule)))) {
                        return FirewallResult::FailedRule;
                    }

                    if (FAILED(rule->put_Name(name.value)) || FAILED(rule->put_ApplicationName(application.value)) ||
                        FAILED(rule->put_Direction(NET_FW_RULE_DIR_OUT)) ||
                        FAILED(rule->put_Action(NET_FW_ACTION_BLOCK)) ||
                        FAILED(rule->put_Protocol(NET_FW_IP_PROTOCOL_ANY)) ||
                        FAILED(rule->put_RemoteAddresses(remote.value)) ||
                        FAILED(rule->put_Profiles(NET_FW_PROFILE2_ALL)) || FAILED(rule->put_Grouping(group.value)) ||
                        FAILED(rule->put_Enabled(VARIANT_TRUE)) || FAILED(rules->Add(rule.Get()))) {
                        return FirewallResult::FailedRule;
                    }
                }

                // Never repair an unexpected existing rule by broadening its scope.
                Bstr actual_app, actual_remote, local_addresses, interface_types, service_name, actual_group;
                NET_FW_RULE_DIRECTION direction{};
                NET_FW_ACTION action{};
                VARIANT_BOOL enabled{};
                long profiles = 0, protocol = 0;

                if (FAILED(rule->get_ApplicationName(actual_app.out())) ||
                    FAILED(rule->get_RemoteAddresses(actual_remote.out())) || FAILED(rule->get_Direction(&direction)) ||
                    FAILED(rule->get_Action(&action)) || FAILED(rule->get_Enabled(&enabled)) ||
                    FAILED(rule->get_Profiles(&profiles)) || FAILED(rule->get_Protocol(&protocol)) ||
                    FAILED(rule->get_LocalAddresses(local_addresses.out())) ||
                    FAILED(rule->get_InterfaceTypes(interface_types.out())) ||
                    FAILED(rule->get_ServiceName(service_name.out())) || FAILED(rule->get_Grouping(actual_group.out()))) {
                    return FirewallResult::FailedRule;
                }

                VARIANT interfaces;
                VariantInit(&interfaces);

                const auto interfaces_result = rule->get_Interfaces(&interfaces);
                const auto unrestricted_interfaces =
                    SUCCEEDED(interfaces_result) && (interfaces.vt == VT_EMPTY || interfaces.vt == VT_NULL);
                VariantClear(&interfaces);

                if (!unrestricted_interfaces || !local_addresses.value || std::wstring(local_addresses.value) != L"*" ||
                    !interface_types.value || _wcsicmp(interface_types.value, L"All") ||
                    (service_name.value && SysStringLen(service_name.value))) {
                    return FirewallResult::FailedRule;
                }

                if (!actual_app.value || !actual_remote.value || _wcsicmp(actual_app.value, canonical.c_str()) ||
                    ranges(actual_remote.value) != ranges(remotes[index]) || direction != NET_FW_RULE_DIR_OUT ||
                    action != NET_FW_ACTION_BLOCK || !actual_group.value ||
                    std::wstring(actual_group.value) != L"Aries offline containment" || profiles != NET_FW_PROFILE2_ALL ||
                    protocol != NET_FW_IP_PROTOCOL_ANY) {
                    return FirewallResult::FailedRule;
                }

                // A reboot leaves matching rules disabled. Only explicit setup
                // may re-enable them, after all scope/ownership checks pass.
                if (enabled != VARIANT_TRUE) {
                    if (!install || FAILED(rule->put_Enabled(VARIANT_TRUE)) ||
                        FAILED(rule->get_Enabled(&enabled)) || enabled != VARIANT_TRUE) {
                        return FirewallResult::FailedRule;
                    }
                }
            }

            return FirewallResult::Success;
        }
        catch (...) {
            return FirewallResult::FailedRule;
        }
    }

    auto allow_loopback(const std::wstring& exe_path) -> FirewallResult {
        return contain_loopback(exe_path, true);
    }

    auto allow_host(const std::wstring& exe_path, const std::wstring& host_ip) -> FirewallResult {
        if (host_ip != L"localhost" && host_ip != L"127.0.0.1" && host_ip != L"::1") {
            return FirewallResult::FailedRule;
        }

        return allow_loopback(exe_path);
    }
} // namespace loader::init::networking
