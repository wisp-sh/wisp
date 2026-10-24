// Snapshot: the compiler sandbox include is relative to this checkout.
#include "wisp.h"
#include "../../cpp/arithmetic.cpp"
extern "C" __attribute__((export_name("demo:cpp-arithmetic/api@0.3.0#add"))) __UINT32_TYPE__ __wisp_export_0(__UINT32_TYPE__ a0, __UINT32_TYPE__ a1) { return add(a0, a1); }
