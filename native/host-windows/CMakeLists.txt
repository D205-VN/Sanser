cmake_minimum_required(VERSION 3.20)

project(sanser_host_windows VERSION 2.0.7 LANGUAGES CXX)

set(SANSER_APP_VERSION "${PROJECT_VERSION}" CACHE STRING "Sanser product version")

include(CTest)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../protocol ${CMAKE_BINARY_DIR}/snv2-protocol)

add_executable(sanser-host-windows
  src/main.cpp
  src/desktop_duplication.cpp
  src/mf_video_encoder.cpp
  src/mf_video_packet_encoder.cpp
  src/bmp_writer.cpp
)

target_compile_definitions(sanser-host-windows PRIVATE
  NOMINMAX
  SANSER_PROTOCOL_VERSION=2
  SANSER_VERSION="${SANSER_APP_VERSION}"
  WIN32_LEAN_AND_MEAN
)

target_include_directories(sanser-host-windows PRIVATE src ${CMAKE_CURRENT_SOURCE_DIR}/../common)

if(BUILD_TESTING)
  add_executable(sanser-wheel-delta-test ../common/wheel_delta_test.cpp)
  target_include_directories(sanser-wheel-delta-test PRIVATE ../common)
  add_test(NAME sanser-wheel-delta-test COMMAND sanser-wheel-delta-test)
endif()

target_link_libraries(sanser-host-windows PRIVATE
  sanser-snv2
  avrt
  bcrypt
  d3d11
  dxgi
  mfplat
  mfreadwrite
  mfuuid
  ole32
  strmiids
  user32
  wmcodecdspuuid
  ws2_32
)

if (MSVC)
  target_compile_options(sanser-host-windows PRIVATE /W4 /permissive-)
endif()
