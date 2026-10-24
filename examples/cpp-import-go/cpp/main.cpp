#include <arithmetic.hpp>

unsigned doubled_sum(unsigned a, unsigned b) {
    return arithmetic::add(a, b) * 2;
}

bool negate(bool value) {
    return arithmetic::negate(value);
}

unsigned long long widen(unsigned long long value) {
    return arithmetic::widen(value);
}
