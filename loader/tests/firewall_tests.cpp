// Exercise private normalization and an uninstalled COM rule. This test never
// calls INetFwRules::Add and makes no Windows firewall policy changes.
#include "../src/init/networking/firewall.cpp"

namespace loader::tests {
auto run_firewall_tests() -> void {
    using namespace init::networking;
    if (ranges(L"::/128,::2-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff") != ranges(L"0:0:0:0:0:0:0:0-0:0:0:0:0:0:0:0,0:0:0:0:0:0:0:2-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff")) throw std::runtime_error("IPv6 firewall normalization mismatch");
    if (ranges(L"0.0.0.0-127.0.0.0,127.0.0.2-255.255.255.255") == ranges(L"0.0.0.0-126.255.255.255,127.0.0.2-255.255.255.255")) throw std::runtime_error("firewall hole was not detected");
    const auto initialized = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    if (FAILED(initialized)) throw std::runtime_error("test COM initialization failed");
    struct Scope { ~Scope() { CoUninitialize(); } } scope;
    ComPtr<INetFwRule> rule;
    if (FAILED(CoCreateInstance(__uuidof(NetFwRule), nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&rule)))) throw std::runtime_error("cannot create transient firewall rule");
    Bstr remote(L"::/128,::2-ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff"), actual;
    if (FAILED(rule->put_RemoteAddresses(remote.value)) || FAILED(rule->get_RemoteAddresses(actual.out())) || ranges(actual.value) != ranges(remote.value)) throw std::runtime_error("Windows IPv6 scope round-trip mismatch");
    Bstr local, types, service;
    rule->get_LocalAddresses(local.out()); rule->get_InterfaceTypes(types.out()); rule->get_ServiceName(service.out());
    if (!local.value || std::wstring(local.value) != L"*" || !types.value || _wcsicmp(types.value, L"All") || (service.value && SysStringLen(service.value))) throw std::runtime_error("transient firewall defaults differ from verification assumptions");
    VARIANT interfaces; VariantInit(&interfaces);
    const auto result = rule->get_Interfaces(&interfaces);
    const auto unrestricted = SUCCEEDED(result) && (interfaces.vt == VT_EMPTY || interfaces.vt == VT_NULL);
    VariantClear(&interfaces);
    if (!unrestricted) throw std::runtime_error("transient firewall interface defaults differ");
}
}
