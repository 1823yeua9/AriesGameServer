#pragma once

#include "../internal/overlay/vendor/imgui/imgui.h"
#include <algorithm>
#include <cctype>
#include <cmath>
#include <string>

// drawn after spt's legacy launcher (dark theme, App.axaml / Styles.axaml):
// near-black cards, blue accent buttons and links, tan secondary text
namespace launcher::widgets {
    enum class ButtonStyle { Normal, Accent, Link, Window, Close };
    enum class Icon { None, Minimize, Close, Gear };
    enum class Align { Left, Center, Right };

    // imgui 1.91 has no dynamic font sizes: each size is its own atlas font
    struct Fonts {
        ImFont* body = nullptr;
        ImFont* caption = nullptr;
        ImFont* label = nullptr;
        ImFont* button = nullptr;
        ImFont* heading = nullptr;
        ImFont* title = nullptr;
    };

    inline Fonts fonts;

    inline auto color(int red, int green, int blue, int alpha = 255) -> ImU32 {
        return IM_COL32(red, green, blue, static_cast<int>(alpha * ImGui::GetStyle().Alpha));
    }

    namespace palette {
        // spt BackgroundBrush, BackgroundBrush2, AltBackgroundBrush
        inline auto card() -> ImU32 { return color(0x18, 0x1a, 0x1b, 215); }
        inline auto raised() -> ImU32 { return color(0x39, 0x3e, 0x41); }
        inline auto backdrop() -> ImU32 { return color(0x37, 0x33, 0x2d, 70); }
        // AccentBrush 1-3 (blue), AltAccentBrush 1-3 (tan)
        inline auto accent() -> ImU32 { return color(0x54, 0xa4, 0xff); }
        inline auto accent_hover() -> ImU32 { return color(0x43, 0x83, 0xcc); }
        inline auto accent_pressed() -> ImU32 { return color(0x32, 0x62, 0x99); }
        inline auto tan() -> ImU32 { return color(0xb2, 0xa5, 0x8d); }
        inline auto tan_dim() -> ImU32 { return color(0x7f, 0x75, 0x64); }
        inline auto ink() -> ImU32 { return color(255, 255, 255); }
        inline auto soft() -> ImU32 { return color(200, 204, 206); }
        inline auto good() -> ImU32 { return color(110, 200, 130); }
        inline auto idle() -> ImU32 { return color(0x5a, 0x60, 0x63); }
        inline auto bad() -> ImU32 { return color(0xcd, 0x5c, 0x5c); }
    } // namespace palette

    inline auto window_origin() -> ImVec2 {
        return ImGui::GetWindowPos();
    }

    /// text in a region: aligned horizontally, wrapped, the block centered vertically.
    inline auto text_in(const char* text, ImVec2 start, ImVec2 size, ImU32 tint, ImFont* font = nullptr, Align align = Align::Center) -> void {
        font = font ? font : ImGui::GetFont();
        const auto pixels = font->FontSize;
        auto* draw = ImGui::GetWindowDrawList();
        const auto* end = text + std::char_traits<char>::length(text);
        draw->PushClipRect(start, {start.x + size.x, start.y + size.y}, true);

        int lines = 0;
        for (const char* scan = text; *scan && lines < 64;) {
            const auto* line_end = font->CalcWordWrapPositionA(1, scan, end, size.x);
            scan = line_end == scan ? scan + 1 : line_end;
            while (*scan == ' ' || *scan == '\n') {
                ++scan;
            }
            ++lines;
        }
        const auto line_height = std::floor(pixels * 1.3f);
        const auto block = std::min(size.y, lines * line_height - (line_height - pixels));
        float y = std::floor(start.y + (size.y - block) / 2);
        const char* cursor = text;

        while (*cursor && y + pixels <= start.y + size.y + 1) {
            const auto* line_end = font->CalcWordWrapPositionA(1, cursor, end, size.x);
            if (line_end == cursor) {
                line_end = cursor + 1;
                while (line_end < end && (static_cast<unsigned char>(*line_end) & 0xc0) == 0x80) {
                    ++line_end;
                }
            }
            const auto width = font->CalcTextSizeA(pixels, FLT_MAX, 0, cursor, line_end).x;
            const auto x = align == Align::Left ? start.x : align == Align::Right ? start.x + size.x - width : start.x + (size.x - width) / 2;
            draw->AddText(font, pixels, {std::floor(x), y}, tint, cursor, line_end);
            y += line_height;
            cursor = line_end;
            while (*cursor == ' ' || *cursor == '\n') {
                ++cursor;
            }
        }

        draw->PopClipRect();
    }

    /// text_in with window-relative coordinates
    inline auto label(const char* text, ImVec2 position, ImVec2 size, ImU32 tint, ImFont* font = nullptr, Align align = Align::Left) -> void {
        const auto origin = window_origin();
        text_in(text, {origin.x + position.x, origin.y + position.y}, size, tint, font, align);
    }

    inline auto text_width(const char* text, ImFont* font) -> float {
        return font->CalcTextSizeA(font->FontSize, FLT_MAX, 0, text).x;
    }

    /// spt's Classes="card": dark rounded box with a soft drop shadow.
    inline auto card(ImVec2 position, ImVec2 size, bool lifted = false) -> void {
        auto* draw = ImGui::GetWindowDrawList();
        const auto origin = window_origin();
        const ImVec2 start{origin.x + position.x, origin.y + position.y};
        const ImVec2 end{start.x + size.x, start.y + size.y};
        for (int ring = 1; ring <= 4; ++ring) {
            const auto spread = static_cast<float>(ring * 2);
            draw->AddRectFilled({start.x - spread / 2, start.y + spread}, {end.x + spread / 2, end.y + spread + 2}, color(0, 0, 0, 22), 5 + spread);
        }
        draw->AddRectFilled(start, end, palette::card(), 5);
        if (lifted) {
            draw->AddRect(start, end, palette::accent(), 5, 0, 1.5f);
        }
    }

    inline auto gear(ImDrawList* draw, ImVec2 center, float radius, ImU32 tint) -> void {
        for (int tooth = 0; tooth < 8; ++tooth) {
            const auto angle = tooth * 3.14159265f / 4;
            const ImVec2 direction{std::cos(angle), std::sin(angle)};
            draw->AddLine({center.x + direction.x * radius * 0.7f, center.y + direction.y * radius * 0.7f},
                          {center.x + direction.x * radius * 1.15f, center.y + direction.y * radius * 1.15f},
                          tint,
                          2.2f);
        }
        draw->AddCircle(center, radius * 0.72f, tint, 16, 1.8f);
        draw->AddCircle(center, radius * 0.28f, tint, 12, 1.4f);
    }

    inline auto button(const char* text, ImVec2 size, ButtonStyle style = ButtonStyle::Normal, Icon icon = Icon::None) -> bool {
        const auto start = ImGui::GetCursorScreenPos();
        const auto clicked = ImGui::InvisibleButton(text, size, ImGuiButtonFlags_EnableNav);
        const auto hovered = ImGui::IsItemHovered();
        const auto active = ImGui::IsItemActive();
        auto* draw = ImGui::GetWindowDrawList();
        const ImVec2 end{start.x + size.x, start.y + size.y};
        auto ink = palette::ink();
        auto* font = fonts.button;

        switch (style) {
        case ButtonStyle::Accent:
            draw->AddRectFilled(start, end, active ? palette::accent_pressed() : hovered ? palette::accent_hover() : palette::accent(), 5);
            break;
        case ButtonStyle::Normal:
            draw->AddRectFilled(start, end, active ? palette::accent_pressed() : hovered ? color(0x45, 0x4b, 0x4f) : palette::raised(), 5);
            draw->AddRect(start, end, hovered ? palette::accent() : color(0x54, 0xa4, 0xff, 90), 5);
            break;
        case ButtonStyle::Link:
            ink = active ? palette::accent_pressed() : hovered ? palette::accent_hover() : palette::accent();
            font = fonts.caption;
            break;
        case ButtonStyle::Window:
            if (hovered) {
                draw->AddRectFilled(start, end, active ? palette::accent_pressed() : palette::accent_hover());
            }
            ink = palette::tan();
            break;
        case ButtonStyle::Close:
            if (hovered) {
                draw->AddRectFilled(start, end, active ? color(220, 20, 60) : palette::bad());
            }
            ink = hovered ? palette::ink() : palette::tan();
            break;
        }

        if (ImGui::IsItemFocused() && ImGui::GetIO().NavVisible) {
            draw->AddRect({start.x - 2, start.y - 2}, {end.x + 2, end.y + 2}, color(0x54, 0xa4, 0xff, 200), 7, 0, 1.5f);
        }

        const ImVec2 center{std::floor(start.x + size.x / 2) + 0.5f, std::floor(start.y + size.y / 2) + 0.5f};
        if (icon == Icon::Minimize) {
            draw->AddLine({center.x - 6, center.y}, {center.x + 6, center.y}, ink, 1.4f);
        }
        else if (icon == Icon::Close) {
            draw->AddLine({center.x - 5, center.y - 5}, {center.x + 5, center.y + 5}, ink, 1.4f);
            draw->AddLine({center.x + 5, center.y - 5}, {center.x - 5, center.y + 5}, ink, 1.4f);
        }
        else if (icon == Icon::Gear) {
            // spt: a gear and the word, as one link
            const auto width = text_width(text, font) + 20;
            const auto left = std::floor(start.x + (size.x - width) / 2);
            gear(draw, {left + 7, center.y}, 6, ink);
            text_in(text, {left + 20, start.y}, {width - 20, size.y}, ink, font, Align::Left);
            if (hovered) {
                draw->AddLine({left + 20, center.y + font->FontSize / 2 + 1}, {left + width, center.y + font->FontSize / 2 + 1}, ink);
            }
        }
        else {
            text_in(text, {start.x + 6, start.y}, {size.x - 12, size.y}, ink, font);
            if (style == ButtonStyle::Link && hovered) {
                const auto width = text_width(text, font);
                draw->AddLine({center.x - width / 2, center.y + font->FontSize / 2 + 1}, {center.x + width / 2, center.y + font->FontSize / 2 + 1}, ink);
            }
        }

        return clicked;
    }

    /// a button at a window-relative position
    inline auto button_at(const char* text, ImVec2 position, ImVec2 size, ButtonStyle style = ButtonStyle::Normal, Icon icon = Icon::None) -> bool {
        ImGui::SetCursorPos(position);
        return button(text, size, style, icon);
    }

    /// spt side image stand-in: a rounded tile with the player's initial.
    inline auto avatar(const char* name, ImVec2 position, float size) -> void {
        auto* draw = ImGui::GetWindowDrawList();
        const auto origin = window_origin();
        const ImVec2 start{origin.x + position.x, origin.y + position.y};
        draw->AddRectFilledMultiColor(start, {start.x + size, start.y + size}, color(0x43, 0x83, 0xcc), color(0x32, 0x62, 0x99), color(0x24, 0x47, 0x70), color(0x32, 0x62, 0x99));
        draw->AddRect(start, {start.x + size, start.y + size}, color(255, 255, 255, 40), 0);
        char initial[2] = {name && *name ? static_cast<char>(std::toupper(static_cast<unsigned char>(*name))) : '?', 0};
        if (static_cast<unsigned char>(initial[0]) >= 0x80) {
            initial[0] = '?';
        }
        text_in(initial, start, {size, size}, palette::ink(), size >= 50 ? fonts.title : fonts.heading);
    }

    /// spt ProgressBar: dark track, accent fill
    inline auto progress(ImVec2 position, ImVec2 size, float value) -> void {
        auto* draw = ImGui::GetWindowDrawList();
        const auto origin = window_origin();
        const ImVec2 start{origin.x + position.x, origin.y + position.y};
        draw->AddRectFilled(start, {start.x + size.x, start.y + size.y}, palette::raised(), size.y / 2);
        if (value > 0) {
            draw->AddRectFilled(start, {start.x + std::max(size.y, size.x * std::clamp(value, 0.0f, 1.0f)), start.y + size.y}, palette::accent(), size.y / 2);
        }
    }

    /// a status dot and its word, left aligned at a window-relative position
    inline auto indicator(const char* text, bool on, ImVec2 position) -> void {
        auto* draw = ImGui::GetWindowDrawList();
        const auto origin = window_origin();
        const ImVec2 start{origin.x + position.x, origin.y + position.y};
        draw->AddCircleFilled({start.x + 4, start.y + fonts.caption->FontSize / 2 + 1}, 4, on ? palette::good() : palette::idle(), 12);
        draw->AddText(fonts.caption, fonts.caption->FontSize, {start.x + 14, start.y}, on ? palette::soft() : palette::tan_dim(), text);
    }
} // namespace launcher::widgets
