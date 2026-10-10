cmake_minimum_required(VERSION 3.20)

project(sanser_host_windows VERSION 2.1.21 LANGUAGES CXX)

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
  src/gpu_video_processor.cpp
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
  add_executable(sanser-control-timing-test ../common/control_timing_test.cpp)
  add_test(NAME sanser-control-timing-test COMMAND sanser-control-timing-test)
  add_executable(sanser-stream-adaptation-test ../common/stream_adaptation_policy_test.cpp)
  add_test(NAME sanser-stream-adaptation-test COMMAND sanser-stream-adaptation-test)
  add_executable(sanser-precise-wait-test ../common/windows_precise_wait_test.cpp)
  add_test(NAME sanser-precise-wait-test COMMAND sanser-precise-wait-test)
  set_tests_properties(sanser-precise-wait-test PROPERTIES TIMEOUT 10)
  add_executable(sanser-wheel-delta-test ../common/wheel_delta_test.cpp)
  target_include_directories(sanser-wheel-delta-test PRIVATE ../common)
  add_test(NAME sanser-wheel-delta-test COMMAND sanser-wheel-delta-test)
  add_executable(sanser-mf-encoder-test
    src/mf_encoder_test.cpp src/mf_video_packet_encoder.cpp src/gpu_video_processor.cpp
    src/mf_video_encoder.cpp src/desktop_duplication.cpp)
  target_compile_definitions(sanser-mf-encoder-test PRIVATE NOMINMAX WIN32_LEAN_AND_MEAN)
  target_link_libraries(sanser-mf-encoder-test PRIVATE mfplat mfreadwrite mfuuid ole32 strmiids wmcodecdspuuid d3d11 d3dcompiler dxgi user32)
  add_test(NAME sanser-mf-encoder-test COMMAND sanser-mf-encoder-test)
  set_tests_properties(sanser-mf-encoder-test PROPERTIES TIMEOUT 30)
endif()

target_link_libraries(sanser-host-windows PRIVATE
  avrt
  bcrypt
  d3d11
  d3dcompiler
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

add_subdirectory(${CMAKE_CURRENT_SOURCE_DIR}/../desktop ${CMAKE_BINARY_DIR}/cross-platform)
target_link_libraries(sanser-host-windows PRIVATE sanser-desktop-native)
