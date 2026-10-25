#include "wisp_types.hpp"
#include <expected>
#include <optional>
#include <string>
#include <vector>

struct Packet {
    wisp::string label;
    wisp::bytes data;
    unsigned long long sequence;
    bool enabled;
};

wisp::string echo(wisp::string value) { return value; }
wisp::bytes buffer(wisp::bytes value) { return value; }
Packet packet(Packet value) { return value; }
wisp::result<Packet, wisp::string> checked(bool success) {
    static const unsigned char data[] = {0, 128, 255};
    if (success) {
        return wisp::result<Packet, wisp::string>::success(Packet{wisp::string{"hello 🦀"}, wisp::bytes{data, 3}, 9007199254740994ULL, true});
    }
    return wisp::result<Packet, wisp::string>::failure(wisp::string{"denied"});
}
wisp::result<Packet, wisp::string> relay(wisp::result<Packet, wisp::string> value) { return value; }
wisp::result<float, unsigned> numeric32(wisp::result<float, unsigned> value) { return value; }
wisp::result<double, unsigned long long> numeric64(wisp::result<double, unsigned long long> value) { return value; }

struct Wide {
    wisp::string a, b, c, d, e, f, g, h, i;
};
Wide wide(Wide value) { return value; }

// These ordinary C++ values use upstream-generated list, option, enum and
// result bindings. Native enum values need not match WIT discriminants.
enum class Mode { fast = 11, careful = 42 };
std::string std_echo(std::string value) { return value; }
std::vector<std::string> labels(std::vector<std::string> value) { return value; }
std::optional<Mode> selected(std::optional<Mode> value) { return value; }
std::vector<Mode> modes(std::vector<Mode> value) { return value; }
std::vector<std::optional<Mode>> choices(std::vector<std::optional<Mode>> value) { return value; }
struct Batch {
    std::vector<std::string> labels;
    std::optional<Mode> selected;
    std::vector<Mode> modes;
};
Batch batch(Batch value) { return value; }
std::expected<Packet, std::string> result_value(std::expected<Packet, std::string> value) { return value; }

Packet packet_value(Packet value) { return value; }
Batch batch_value(Batch value) { return value; }
