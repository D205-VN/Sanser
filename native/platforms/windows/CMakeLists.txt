add_library(sanser-native-windows STATIC
  backend.cpp
  ../../host-windows/src/desktop_duplication.cpp
  ../../host-windows/src/mf_video_packet_encoder.cpp
  ../../host-windows/src/mf_video_encoder.cpp
)
add_library(sanser-desktop-native ALIAS sanser-native-windows)
target_link_libraries(sanser-native-windows PUBLIC sanser-desktop-transport)
target_include_directories(sanser-native-windows PRIVATE ../../host-windows/src)
target_compile_definitions(sanser-native-windows PRIVATE NOMINMAX WIN32_LEAN_AND_MEAN)
if(MSVC)
  target_compile_options(sanser-native-windows PRIVATE /utf-8 /W4)
endif()
target_link_libraries(sanser-native-windows PUBLIC d3d11 dxgi mfplat mfreadwrite mfuuid ole32 wmcodecdspuuid user32 gdi32 strmiids)
