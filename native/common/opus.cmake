include_guard(GLOBAL)
include(FetchContent)
if(POLICY CMP0135)
  cmake_policy(SET CMP0135 NEW)
endif()
set(OPUS_BUILD_SHARED_LIBRARY OFF CACHE BOOL "" FORCE)
set(OPUS_BUILD_TESTING OFF CACHE BOOL "" FORCE)
set(OPUS_BUILD_PROGRAMS OFF CACHE BOOL "" FORCE)
set(OPUS_INSTALL_PKG_CONFIG_MODULE OFF CACHE BOOL "" FORCE)
set(OPUS_INSTALL_CMAKE_CONFIG_MODULE OFF CACHE BOOL "" FORCE)
# No model downloads or optional neural codec features for the realtime path.
set(OPUS_DRED OFF CACHE BOOL "" FORCE)
set(OPUS_OSCE OFF CACHE BOOL "" FORCE)
if(APPLE AND CMAKE_SYSTEM_PROCESSOR MATCHES "^(arm64|aarch64)$")
  set(OPUS_MAY_HAVE_NEON OFF CACHE BOOL "" FORCE)
  set(OPUS_PRESUME_NEON ON CACHE BOOL "" FORCE)
endif()
FetchContent_Declare(sanser_opus
  URL https://downloads.xiph.org/releases/opus/opus-1.6.1.tar.gz
  URL_HASH SHA256=6ffcb593207be92584df15b32466ed64bbec99109f007c82205f0194572411a1)
# Upstream promotes BUILD_TESTING into its own tests; scope it to the dependency.
set(_sanser_build_testing ${BUILD_TESTING})
set(BUILD_TESTING OFF)
FetchContent_MakeAvailable(sanser_opus)
set(BUILD_TESTING ${_sanser_build_testing})
if(APPLE AND CMAKE_SYSTEM_PROCESSOR MATCHES "^(arm64|aarch64)$")
  # ARM64 guarantees NEON. Upstream's headers also guard the declarations by
  # MAY_HAVE even when PRESUME is set; no runtime CPU probe is needed here.
  target_compile_definitions(opus PRIVATE OPUS_ARM_MAY_HAVE_NEON OPUS_ARM_MAY_HAVE_NEON_INTR)
endif()
add_library(sanser-audio-codec STATIC ${CMAKE_CURRENT_LIST_DIR}/opus_audio.cpp)
target_compile_features(sanser-audio-codec PUBLIC cxx_std_20)
target_include_directories(sanser-audio-codec PUBLIC ${CMAKE_CURRENT_LIST_DIR})
target_link_libraries(sanser-audio-codec PUBLIC Opus::opus)
