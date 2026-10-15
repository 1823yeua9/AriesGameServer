#include "application.h"

#include <shellapi.h>

#include <fstream>
#include <string_view>

auto WINAPI wWinMain(HINSTANCE, HINSTANCE, PWSTR, int) -> int {
    try {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        int argument_count = 0;
        auto** arguments = CommandLineToArgvW(GetCommandLineW(), &argument_count);

        if (!arguments) {
            return 1;
        }

        const bool self_test = argument_count == 2 && std::wstring_view(arguments[1]) == L"--self-test";
        const bool smoke_test = argument_count == 2 && std::wstring_view(arguments[1]) == L"--smoke-test";
        const bool preview = argument_count == 2 && std::wstring_view(arguments[1]) == L"--preview";
        const bool valid = argument_count == 1 || self_test || smoke_test || preview;
        LocalFree(arguments);

        if (!valid) {
            return 2;
        }

        if (self_test) {
            return launcher::run_blur_tests(launcher::executable_directory() / "blur-artifacts");
        }

        return launcher::Application(smoke_test, preview).run();
    }
    catch (const std::exception& error) {
        std::ofstream log(launcher::executable_directory() / "launcher-error.txt");
        log << error.what() << '\n';

        return 1;
    }
}
