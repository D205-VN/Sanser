cmake_minimum_required(VERSION 3.20)

project(sanser_client_macos VERSION 2.0.7 LANGUAGES CXX OBJCXX)

set(SANSER_APP_VERSION "${PROJECT_VERSION}" CACHE STRING "Sanser product version")

if (NOT APPLE)
  message(FATAL_ERROR "sanser-client-macos is macOS-only.")
endif()

# Keep the native sidecar compatible with the same minimum macOS release as
# the Tauri bundle.  Without an explicit deployment target, a locally-built
# sidecar inherits the current SDK version and can make an otherwise valid app
# fail immediately on older supported Macs.
if (NOT CMAKE_OSX_DEPLOYMENT_TARGET)
  set(CMAKE_OSX_DEPLOYMENT_TARGET "12.0" CACHE STRING "Minimum supported macOS version" FORCE)
endif()

include(CTest)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_OBJCXX_STANDARD 20)
set(CMAKE_OBJCXX_STANDARD_REQUIRED ON)

add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../protocol ${CMAKE_BINARY_DIR}/snv2-protocol)

add_executable(sanser-client-macos
  src/main.mm
)

if(BUILD_TESTING)
  add_executable(sanser-wheel-delta-test ../common/wheel_delta_test.cpp)
  target_include_directories(sanser-wheel-delta-test PRIVATE ../common)
  add_test(NAME sanser-wheel-delta-test COMMAND sanser-wheel-delta-test)
endif()

find_library(COCOA_FRAMEWORK Cocoa REQUIRED)
find_library(AUDIOTOOLBOX_FRAMEWORK AudioToolbox REQUIRED)
find_library(COREAUDIO_FRAMEWORK CoreAudio REQUIRED)
find_library(COREHAPTICS_FRAMEWORK CoreHaptics REQUIRED)
find_library(COREGRAPHICS_FRAMEWORK CoreGraphics REQUIRED)
find_library(COREMEDIA_FRAMEWORK CoreMedia REQUIRED)
find_library(COREVIDEO_FRAMEWORK CoreVideo REQUIRED)
find_library(GAMECONTROLLER_FRAMEWORK GameController REQUIRED)
find_library(METAL_FRAMEWORK Metal REQUIRED)
find_library(METALKIT_FRAMEWORK MetalKit REQUIRED)
find_library(QUARTZCORE_FRAMEWORK QuartzCore REQUIRED)
find_library(VIDEOTOOLBOX_FRAMEWORK VideoToolbox REQUIRED)

target_compile_definitions(sanser-client-macos PRIVATE
  SANSER_PROTOCOL_VERSION=2
  SANSER_VERSION="${SANSER_APP_VERSION}"
)
target_include_directories(sanser-client-macos PRIVATE ${CMAKE_CURRENT_SOURCE_DIR}/../common)

target_link_libraries(sanser-client-macos PRIVATE
  sanser-snv2
  ${AUDIOTOOLBOX_FRAMEWORK}
  ${COCOA_FRAMEWORK}
  ${COREAUDIO_FRAMEWORK}
  ${COREHAPTICS_FRAMEWORK}
  ${COREGRAPHICS_FRAMEWORK}
  ${COREMEDIA_FRAMEWORK}
  ${COREVIDEO_FRAMEWORK}
  ${GAMECONTROLLER_FRAMEWORK}
  ${METAL_FRAMEWORK}
  ${METALKIT_FRAMEWORK}
  ${QUARTZCORE_FRAMEWORK}
  ${VIDEOTOOLBOX_FRAMEWORK}
)

target_compile_options(sanser-client-macos PRIVATE
  $<$<COMPILE_LANGUAGE:OBJCXX>:-fobjc-arc>
  -Wall
  -Wextra
)
