#include "gtest/gtest.h"
#include <cerrno>
#include <leptonica/allheaders.h>

extern "C" int system(const char *) { errno = ENOSYS; return -1; }
extern "C" int sat_upstream_tests(int argc, char **argv) {
    setLeptDebugOK(0);
    setMsgSeverity(L_SEVERITY_ERROR);
    ::testing::InitGoogleTest(&argc,argv);
    return RUN_ALL_TESTS();
}
