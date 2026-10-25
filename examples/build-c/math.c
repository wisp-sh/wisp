#include <stdlib.h>
uint32_t exports_library_add(uint32_t a, uint32_t b) {
    uint32_t *values = malloc(2 * sizeof(uint32_t));
    if (!values) abort();
    values[0] = a; values[1] = b;
    uint32_t result = values[0] + values[1];
    free(values);
    return result;
}
