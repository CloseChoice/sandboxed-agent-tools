#pragma once
#ifdef __wasi__
#include <errno.h>
// Temporary-file helpers are outside this in-memory OCR profile.
static inline int sat_ocr_mkstemp(char *path) {
    (void)path;
    errno = ENOSYS;
    return -1;
}
#define mkstemp sat_ocr_mkstemp
#endif
