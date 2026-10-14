#pragma once
#include <string>
#include <stdexcept>
#include <filesystem>
#include <map>

namespace loader::config {
	constexpr auto settings_file_name = "settings.val"; // in appdata

	class Settings {

		std::string game_path = "";
		std::string game_settings_path = "";

		// This launch profile is isolated; remote hosts are rejected at preflight.
		std::string host_ip = "localhost";
		std::filesystem::path file_path;
		std::map<std::string, std::string> launch_settings = {
			{"port", "58089"},
			{"admin_port", "58090"},
			{"server_path", "server/platform/target/release/aries-platform.exe"},
			{"internal_path", "x64/Release/internal.dll"},
			{"cert_path", "../local_backend/embedded_tls/remoting_cert.pem"},
			{"key_path", "../local_backend/embedded_tls/remoting_key.pem"},
			{"skin_catalog", "../local_backend/skin_catalog.json"},
			{"skin_state", "runtime/account/skins.json"},
			{"runtime_path", "runtime"},
			{"gameplay_path", ""},
			{"gameplay_config", ""}
		};

	public:

		Settings() = default;
		explicit Settings(std::filesystem::path path) : file_path(std::move(path)) {}
		~Settings() = default;

		// load/save %APPDATA%\Aries\settings.val
		auto load_settings() -> bool;
		auto save_settings() -> bool;

		// Keep the existing string-keyed JSON settings format.
		auto set_setting(const std::string& key, const std::string& value) -> void {

			if (key == "game_path") {
				game_path = value;
			}
			else if (key == "game_settings_path") {
				game_settings_path = value;
			}
			else if (key == "host_ip") {
				host_ip = value;
			}
			else {
				launch_settings.at(key) = value;
			}
		}

		auto get_setting(const std::string& key) const -> const std::string& {

			if (key == "game_path") {
				return game_path;
			}
			else if (key == "game_settings_path") {
				return game_settings_path;
			}
			else if (key == "host_ip") {
				return host_ip;
			}
			return launch_settings.at(key);
		}
		[[nodiscard]] auto path_for(const std::string& key) const -> std::filesystem::path;

	};
}
