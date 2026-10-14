#include "application.h"

#include <exception>
#include <iostream>

int wmain(int argc, wchar_t** argv) {
    try {
        return loader::run(argc, argv);
    }
    catch (const std::exception& error) {
        std::cerr << "Aries launch failed: " << error.what() << '\n';

        return 1;
    }
}
