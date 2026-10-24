#pragma once
#ifdef __cplusplus
extern "C" {
#define WISP_BOOL bool
#else
#define WISP_BOOL _Bool
#endif
__UINT32_TYPE__ exports_arithmetic_add(__UINT32_TYPE__ a0, __UINT32_TYPE__ a1);
#ifdef __cplusplus
}
#endif
