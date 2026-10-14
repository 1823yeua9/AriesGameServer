
// entrypoint class essentially does nothing
class entry {
public:

	entry() = default;
	~entry() = default;

	static auto run() -> void {

	}
};

int main() {
	entry::run();
	return 0;
}