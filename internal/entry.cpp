#include "session.h"
#include "startup.h"
#include "overlay/overlay.hpp"
#include "overlay/present_hook.hpp"
#include "overlay/skin_client.hpp"

#include <Windows.h>
#include <fstream>

namespace internal {
    static auto WINAPI run(void* module) -> DWORD {
        try {
            session::initialize();
            startup::prepare(session::runtime_directory());
            session::signal_ready();
            startup::finish(session::runtime_directory());

            skin_client::initialize(static_cast<HMODULE>(module));
            present_hook::set_resize_callback(overlay::on_resize);

            if (!present_hook::install(overlay::on_present)) {
                throw std::runtime_error("Cannot install dev-menu renderer");
            }

            session::log("Dev menu installed; INSERT toggles; lifetime owned by game process");
        }
        catch (const std::exception& error) {
            session::log(std::string("FAILED: ") + error.what());

            if (!session::runtime_directory().empty()) {
                std::ofstream failed(session::runtime_directory() / "internal.failed");
                failed << error.what();
            }

            return 1;
        }

        return 0;
    }
} // namespace internal

auto WINAPI DllMain(HINSTANCE module, DWORD reason, void*) -> BOOL {
    if (reason == DLL_PROCESS_ATTACH) {
        DisableThreadLibraryCalls(module);
        const auto thread = CreateThread(nullptr, 0, internal::run, module, 0, nullptr);

        if (!thread) {
            return FALSE;
        }

        CloseHandle(thread);
    }

    return TRUE;
}
