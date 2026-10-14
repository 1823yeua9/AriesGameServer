#pragma once
#include <string>
#include <stdexcept>

namespace loader::config {
	constexpr auto SETTINGS_FILE_NAME = "settings.val"; // in appdata 

	class Settings {

		std::string game_path = "";
		std::string game_settings_path = "";

		// game profile things id assume, save file and shit like that

		// if online / using radmin 
		std::string host_ip = "localhost"; // we dont set ports since valorant uses about 30 billion of them

	public:

		Settings() = default;
		~Settings() = default; // no allocations so no need for a destructor

		// load the settings from the json file if it exists
		static auto load_settings() -> bool;
		static auto save_settings() -> bool;

		template<typename T>
		static auto set_setting(const std::string& key, const T& value) -> void {
			
			switch (key)
			{
				case "game_path":
					game_path = value;
					break;

				case "game_settings_path":
					game_settings_path = value;
					break;

				case "host_ip":
					host_ip = value;
					break;
				default:
					throw std::invalid_argument("Invalid setting key");
					break;
			}
		}

		template<typename T>
		auto get_setting(const std::string& key) const -> T {

			switch (key)
			{
				case "game_path":
					return this->game_path;
					break;

				case "game_settings_path":
					return this->game_settings_path;
					break;

				case "host_ip":
					return this->host_ip;
					break;
				default:
					throw std::invalid_argument("Invalid setting key");
					break;
			}
		}

	};
}