cmake_minimum_required(VERSION 3.20)

project(sanser_native_client_mac LANGUAGES CXX OBJCXX)

if (NOT APPLE)
  message(FATAL_ERROR "sanser-native-client is macOS-only.")
endif()

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_OBJCXX_STANDARD 20)
set(CMAKE_OBJCXX_STANDARD_REQUIRED ON)

add_executable(sanser-native-client
  src/main.mm
)

find_library(COCOA_FRAMEWORK Cocoa REQUIRED)
find_library(COREGRAPHICS_FRAMEWORK CoreGraphics REQUIRED)
find_library(COREMEDIA_FRAMEWORK CoreMedia REQUIRED)
find_library(COREVIDEO_FRAMEWORK CoreVideo REQUIRED)
find_library(METAL_FRAMEWORK Metal REQUIRED)
find_library(METALKIT_FRAMEWORK MetalKit REQUIRED)
find_library(QUARTZCORE_FRAMEWORK QuartzCore REQUIRED)
find_library(VIDEOTOOLBOX_FRAMEWORK VideoToolbox REQUIRED)

target_link_libraries(sanser-native-client PRIVATE
  ${COCOA_FRAMEWORK}
  ${COREGRAPHICS_FRAMEWORK}
  ${COREMEDIA_FRAMEWORK}
  ${COREVIDEO_FRAMEWORK}
  ${METAL_FRAMEWORK}
  ${METALKIT_FRAMEWORK}
  ${QUARTZCORE_FRAMEWORK}
  ${VIDEOTOOLBOX_FRAMEWORK}
)

target_compile_options(sanser-native-client PRIVATE
  $<$<COMPILE_LANGUAGE:OBJCXX>:-fobjc-arc>
  -Wall
  -Wextra
)
