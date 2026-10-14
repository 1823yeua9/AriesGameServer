#include <iostream>
#include "utils/config/settings.h"


int main() {
	
	loader::config::Settings settings;

	if (!settings.load_settings()) {
		
		// prompt the user to enter the game path and settings path etc

		// then use settings.set_setting("", "") to set the values
	}

	// finish up firewall and other things and put that in networking
	
	// start server & game init/ will hold these

	// initialize injector class and inject our module

	return 0;
}