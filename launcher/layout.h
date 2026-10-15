#pragma once

namespace launcher::layout {
    inline constexpr float window_width = 760;
    inline constexpr float title_height = 48;
    inline constexpr float page_margin = 24;
    inline constexpr float tab_width = 92;
    inline constexpr float caption_button_width = 46;
    inline constexpr float caption_gap = 8;
    inline constexpr float tabs_left = window_width - tab_width * 3 - caption_button_width * 2 - caption_gap;

    constexpr auto is_drag_region(int x, int y) -> bool {
        return x >= 0 && x < tabs_left && y >= 0 && y < title_height;
    }

    static_assert(is_drag_region(383, 47));
    static_assert(!is_drag_region(384, 24)); // Home starts here, not a drag handle.
    static_assert(!is_drag_region(100, 48));
    static_assert(!is_drag_region(-1, 20));
} // namespace launcher::layout
