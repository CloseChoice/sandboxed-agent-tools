#pragma once
#include "include_gunit.h"
#ifdef SAT_TEST_TMPDIR
// The upstream helper's pointer is mutable. Give its original file operations
// an absolute, explicitly granted temporary directory without editing tests.
static struct SatUpstreamTemporaryDirectory {
    SatUpstreamTemporaryDirectory() { FLAGS_test_tmpdir = SAT_TEST_TMPDIR; }
} sat_upstream_temporary_directory;
#endif
