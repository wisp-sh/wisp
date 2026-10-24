// Snapshot: the compiler sandbox include is relative to this checkout.
#include "wisp.h"
#include "../../cpp/main.cpp"
extern "C" __attribute__((export_name("demo:cpp-import-go/api@0.1.0#doubled-sum"))) __UINT32_TYPE__ __wisp_export_0(__UINT32_TYPE__ a0, __UINT32_TYPE__ a1) { return doubled_sum(a0, a1); }
extern "C" __attribute__((export_name("demo:cpp-import-go/api@0.1.0#negate"))) WISP_BOOL __wisp_export_1(WISP_BOOL a0) { return negate(a0); }
extern "C" __attribute__((export_name("demo:cpp-import-go/api@0.1.0#widen"))) __UINT64_TYPE__ __wisp_export_2(__UINT64_TYPE__ a0) { return widen(a0); }
