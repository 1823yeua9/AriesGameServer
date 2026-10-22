#include "firewall_reset.h"

#include <Windows.h>
#include <taskschd.h>
#include <wrl/client.h>
#include <string>

#pragma comment(lib, "taskschd.lib")

namespace loader::init::networking {
    using Microsoft::WRL::ComPtr;

    static constexpr auto reset_name = L"Aries Offline Firewall Reset";
    static constexpr auto reset_source = L"Aries.Loader.FirewallReset.v1";

    // No project executable/script is run as SYSTEM. The action uses the Windows
    // PowerShell binary and a fixed command scoped to our rule group AND names.
    static constexpr auto reset_arguments = L"-NoLogo -NoProfile -NonInteractive -WindowStyle Hidden -Command \""
                                            L"$ErrorActionPreference='Stop'; try { "
                                            L"$policy=New-Object -ComObject HNetCfg.FwPolicy2; "
                                            L"foreach($rule in @($policy.Rules)) { "
                                            L"if($rule.Grouping -eq 'Aries offline containment' -and "
                                            L"$rule.Name -match '^Aries offline [0-9]+ block [01]$') { "
                                            L"$rule.Enabled=$false } }; exit 0 } catch { exit 1 }\"";

    class TaskString {
    public:
        BSTR value = nullptr;

        explicit TaskString(const std::wstring& text = {}) : value(SysAllocString(text.c_str())) {}

        ~TaskString() {
            SysFreeString(value);
        }

        auto out() -> BSTR* {
            SysFreeString(value);
            value = nullptr;

            return &value;
        }

        auto equals(const std::wstring& text) const -> bool {
            return value && text == value;
        }

        TaskString(const TaskString&) = delete;
        auto operator=(const TaskString&) -> TaskString& = delete;
    };

    static auto escape_xml(const std::wstring& text) -> std::wstring {
        std::wstring result;

        for (const auto character : text) {
            switch (character) {
            case L'&':
                result += L"&amp;";
                break;
            case L'<':
                result += L"&lt;";
                break;
            case L'>':
                result += L"&gt;";
                break;
            default:
                result += character;
                break;
            }
        }

        return result;
    }

    static auto owned_task(ITaskDefinition* definition) -> bool {
        ComPtr<IRegistrationInfo> information;
        TaskString source;

        return SUCCEEDED(definition->get_RegistrationInfo(&information)) &&
               SUCCEEDED(information->get_Source(source.out())) && source.equals(reset_source);
    }

    static auto verify_task(IRegisteredTask* task, const std::wstring& powershell_path) -> bool {
        ComPtr<ITaskDefinition> definition;
        ComPtr<IPrincipal> principal;
        ComPtr<IActionCollection> actions;
        ComPtr<IAction> action;
        ComPtr<IExecAction> executable;
        ComPtr<ITriggerCollection> triggers;
        ComPtr<ITrigger> trigger;
        TaskString user, path, arguments;
        VARIANT_BOOL enabled = VARIANT_FALSE;
        TASK_LOGON_TYPE logon{};
        TASK_TRIGGER_TYPE2 trigger_type{};
        long count = 0;

        if (FAILED(task->get_Enabled(&enabled)) || enabled != VARIANT_TRUE ||
            FAILED(task->get_Definition(&definition)) || !owned_task(definition.Get()) ||
            FAILED(definition->get_Principal(&principal)) || FAILED(principal->get_UserId(user.out())) ||
            (!user.equals(L"SYSTEM") && !user.equals(L"S-1-5-18")) || FAILED(principal->get_LogonType(&logon)) ||
            logon != TASK_LOGON_SERVICE_ACCOUNT) {
            return false;
        }

        if (FAILED(definition->get_Actions(&actions)) || FAILED(actions->get_Count(&count)) || count != 1 ||
            FAILED(actions->get_Item(1, &action)) || FAILED(action.As(&executable)) ||
            FAILED(executable->get_Path(path.out())) || !path.equals(powershell_path) ||
            FAILED(executable->get_Arguments(arguments.out())) || !arguments.equals(reset_arguments)) {
            return false;
        }

        return SUCCEEDED(definition->get_Triggers(&triggers)) && SUCCEEDED(triggers->get_Count(&count)) && count == 1 &&
               SUCCEEDED(triggers->get_Item(1, &trigger)) && SUCCEEDED(trigger->get_Type(&trigger_type)) &&
               trigger_type == TASK_TRIGGER_BOOT && SUCCEEDED(trigger->get_Enabled(&enabled)) &&
               enabled == VARIANT_TRUE;
    }

    auto ensure_firewall_reset(bool install) -> bool {
        wchar_t system_directory[MAX_PATH]{};
        const auto length = GetSystemDirectoryW(system_directory, MAX_PATH);

        if (!length || length >= MAX_PATH) {
            return false;
        }

        const auto powershell_path = std::wstring(system_directory) + L"\\WindowsPowerShell\\v1.0\\powershell.exe";
        ComPtr<ITaskService> service;
        ComPtr<ITaskFolder> folder;
        ComPtr<IRegisteredTask> task;
        TaskString root(L"\\"), name(reset_name);
        const VARIANT empty{};

        if (FAILED(CoCreateInstance(CLSID_TaskScheduler, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&service))) ||
            FAILED(service->Connect(empty, empty, empty, empty)) || FAILED(service->GetFolder(root.value, &folder))) {
            return false;
        }

        const auto existing = folder->GetTask(name.value, &task);

        if (!install) {
            return SUCCEEDED(existing) && verify_task(task.Get(), powershell_path);
        }

        if (SUCCEEDED(existing)) {
            ComPtr<ITaskDefinition> definition;

            if (FAILED(task->get_Definition(&definition)) || !owned_task(definition.Get())) {
                return false;
            }
        }
        else if (existing != HRESULT_FROM_WIN32(ERROR_FILE_NOT_FOUND)) {
            return false;
        }

        const auto xml =
            std::wstring(L"<Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">") +
            L"<RegistrationInfo><Description>Disable only Aries offline containment rules at Windows startup.</Description>"
            L"<Source>" +
            reset_source +
            L"</Source></RegistrationInfo>"
            L"<Triggers><BootTrigger><Enabled>true</Enabled></BootTrigger></Triggers>"
            L"<Principals><Principal id=\"System\"><UserId>S-1-5-18</UserId><RunLevel>HighestAvailable</RunLevel></Principal></Principals>"
            L"<Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"
            L"<DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>"
            L"<StartWhenAvailable>true</StartWhenAvailable><Enabled>true</Enabled><ExecutionTimeLimit>PT2M</ExecutionTimeLimit>"
            L"<RestartOnFailure><Interval>PT1M</Interval><Count>3</Count></RestartOnFailure></Settings>"
            L"<Actions Context=\"System\"><Exec><Command>" +
            escape_xml(powershell_path) + L"</Command><Arguments>" + escape_xml(reset_arguments) +
            L"</Arguments></Exec></Actions></Task>";

        // Normal launcher preflight may read the task, but only administrators
        // and SYSTEM may modify or run it.
        TaskString task_xml(xml), system(L"SYSTEM"), security(L"D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GR;;;AU)");
        VARIANT account{};
        account.vt = VT_BSTR;
        account.bstrVal = system.value;
        VARIANT descriptor{};
        descriptor.vt = VT_BSTR;
        descriptor.bstrVal = security.value;
        task.Reset();

        return SUCCEEDED(folder->RegisterTask(name.value,
                                              task_xml.value,
                                              TASK_CREATE_OR_UPDATE,
                                              account,
                                              empty,
                                              TASK_LOGON_SERVICE_ACCOUNT,
                                              descriptor,
                                              &task)) &&
               SUCCEEDED(task->SetSecurityDescriptor(security.value, 0)) &&
               verify_task(task.Get(), powershell_path);
    }
} // namespace loader::init::networking
