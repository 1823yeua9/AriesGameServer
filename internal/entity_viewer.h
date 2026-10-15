#pragma once

#include <Windows.h>

namespace internal::entity_viewer {
    auto attach(HWND window) -> bool;

    auto draw_options() -> void;

    auto draw_world() -> void;
} // namespace internal::entity_viewer
