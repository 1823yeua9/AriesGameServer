#include "bot_actions.h"
#include "session.h"
#include "overlay/vendor/imgui/imgui.h"
#include "../loader/3p/json/json.hpp"

#include <Windows.h>
#include <atomic>
#include <chrono>
#include <fstream>
#include <future>
#include <thread>

namespace internal::bot_actions {
    static std::future<std::string> pending;
    static std::string status = "Adds one bot 5 m ahead at your current height, facing you.";
    static std::atomic_uint64_t next_request = 0;

    static auto request_bot(const std::filesystem::path& directory) -> std::string {
        // Only this loader's server consumes this run-local directory. Never use
        // the legacy shared queue or carry a request into a later launch.
        const auto queue = directory / "gameplay_commands";
        const auto name = "add-bot-" + std::to_string(GetTickCount64()) + "-" + std::to_string(++next_request);
        const auto request = queue / (name + ".json");
        const auto temporary = queue / (name + ".tmp");
        const auto response = queue / (name + ".result");

        try {
            if (!std::filesystem::is_directory(queue)) {
                return "Restart through the updated loader to enable Add bot.";
            }

            const auto now = std::chrono::duration_cast<std::chrono::milliseconds>(
                                 std::chrono::system_clock::now().time_since_epoch())
                                 .count();
            const auto payload = nlohmann::json{{"add_bot", true}, {"expires_at_ms", now + 8000}}.dump();
            std::ofstream output(temporary, std::ios::binary);
            output << payload;
            output.close();

            if (!output) {
                throw std::runtime_error("Cannot write bot request");
            }

            std::filesystem::rename(temporary, request);
            const auto deadline = std::chrono::steady_clock::now() + std::chrono::seconds(10);

            while (std::chrono::steady_clock::now() < deadline) {
                if (std::filesystem::is_regular_file(response)) {
                    if (std::filesystem::file_size(response) > 4096) {
                        throw std::runtime_error("Invalid bot response size");
                    }

                    std::ifstream input(response);
                    const auto result = nlohmann::json::parse(input);
                    input.close();
                    std::filesystem::remove(response);
                    return result.at("ok").get<bool>() ? result.at("message").get<std::string>()
                                                       : "Not spawned: " + result.at("message").get<std::string>();
                }

                std::this_thread::sleep_for(std::chrono::milliseconds(100));
            }

            std::error_code error;
            std::filesystem::remove(request, error);
            return "No server confirmation. Enter the playable Range and try again.";
        }
        catch (const std::exception& error) {
            std::error_code ignored;
            std::filesystem::remove(temporary, ignored);
            std::filesystem::remove(request, ignored);
            return std::string("Add bot failed: ") + error.what();
        }
    }

    auto draw() -> void {
        if (pending.valid() && pending.wait_for(std::chrono::seconds(0)) == std::future_status::ready) {
            status = pending.get();
            session::log("Add bot: " + status);
        }

        ImGui::BeginDisabled(pending.valid());

        if (ImGui::Button("Add bot")) {
            try {
                pending = std::async(std::launch::async, request_bot, session::runtime_directory());
                status = "Waiting for the local gameplay server...";
            }
            catch (const std::exception& error) {
                status = error.what();
            }
        }

        ImGui::EndDisabled();
        ImGui::TextWrapped("%s", status.c_str());
    }
} // namespace internal::bot_actions
