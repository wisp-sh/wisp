#include "wisp_types.hpp"

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
