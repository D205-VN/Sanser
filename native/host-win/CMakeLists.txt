cmake_minimum_required(VERSION 3.20)

project(sanser_native_host_win LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_executable(sanser-native-host
  src/main.cpp
  src/desktop_duplication.cpp
  src/mf_video_encoder.cpp
  src/mf_video_packet_encoder.cpp
  src/bmp_writer.cpp
)

target_compile_definitions(sanser-native-host PRIVATE
  NOMINMAX
  WIN32_LEAN_AND_MEAN
)

target_include_directories(sanser-native-host PRIVATE src)

target_link_libraries(sanser-native-host PRIVATE
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
  target_compile_options(sanser-native-host PRIVATE /W4 /permissive-)
endif()
