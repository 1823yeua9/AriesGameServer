#pragma once

#include "runtime.h"
#include "../utils/config/settings.h"

#include <optional>

namespace loader {
    struct ServicePorts {
        unsigned short platform;
        unsigned short admin;
    };

    // Owns one launch's files, credentials and child processes. The application
    // retains its single-instance mutex and pinned executable throughout run().
    class LaunchSession {
    public:
        LaunchSession(const config::Settings& settings, ServicePorts requested_ports, bool server_only);

        auto run() -> void;

    private:
        auto prepare_platform_config() -> void;

        auto start_platform() -> void;

        auto verify_platform_state() -> void;

        auto supervise_platform() -> void;

        auto game_arguments() const -> std::vector<std::wstring>;

        auto start_game() -> void;

        auto initialize_internal() -> void;

        auto start_gameplay() -> void;

        auto request_range() -> void;

        auto supervise_game() -> void;

        const config::Settings& settings;
        ServicePorts requested_ports;
        ServicePorts ports{};
        bool server_only;

        std::filesystem::path run_directory;
        std::filesystem::path gate;
        std::filesystem::path waiting;
        std::filesystem::path state_path;
        std::filesystem::path server_config;
        std::filesystem::path game_log;

        std::string auth_token;
        std::string admin_token;

        init::Job job;
        std::optional<init::Process> server;
        std::optional<init::Process> game;
        std::optional<init::Process> gameplay;
        std::optional<init::Handle> internal_ready;
    };
} // namespace loader
