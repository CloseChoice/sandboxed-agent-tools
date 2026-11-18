cmake_minimum_required(VERSION 3.16)
project(sat_upstream_gtest LANGUAGES C CXX)
set(gtest_disable_pthreads ON CACHE BOOL "No pthreads in the WASI test runner" FORCE)
set(BUILD_GMOCK ON CACHE BOOL "Build upstream GoogleMock" FORCE)
add_subdirectory("${GTEST_SOURCE}" upstream)
foreach(name gtest gmock gtest_main gmock_main)
  # The pinned older GoogleTest sets -fexceptions and guesses pthreads from a
  # link-only probe. Configure its build flags without changing its sources.
  set_target_properties(${name} PROPERTIES COMPILE_FLAGS
    "-fno-exceptions -D_WASI_EMULATED_SIGNAL -DGTEST_HAS_PTHREAD=0 -DGTEST_HAS_STREAM_REDIRECTION=0 -DGTEST_HAS_POSIX_RE=0 -DGTEST_HAS_EXCEPTIONS=0")
endforeach()
