include_guard(GLOBAL)
include(FetchContent)
if(POLICY CMP0135)
  cmake_policy(SET CMP0135 NEW)
endif()
# SpeexDSP 1.2.1, BSD: only the portable floating-point AEC/preprocessor and
# bundled KissFFT. No device access, runtime downloads or voice recordings.
enable_language(C)
FetchContent_Declare(sanser_speexdsp
  URL https://downloads.xiph.org/releases/speex/speexdsp-1.2.1.tar.gz
  URL_HASH SHA256=8c777343e4a6399569c72abc38a95b24db56882c83dbdb6c6424a5f4aeb54d3d)
FetchContent_MakeAvailable(sanser_speexdsp)
set(_speex_generated "${CMAKE_CURRENT_BINARY_DIR}/speex-include")
file(MAKE_DIRECTORY "${_speex_generated}/speex")
file(WRITE "${_speex_generated}/speex/speexdsp_config_types.h"
  "#pragma once\n#include <stdint.h>\ntypedef int16_t spx_int16_t;\ntypedef uint16_t spx_uint16_t;\ntypedef int32_t spx_int32_t;\ntypedef uint32_t spx_uint32_t;\n")
add_library(sanser-speex-echo STATIC
  ${sanser_speexdsp_SOURCE_DIR}/libspeexdsp/mdf.c
  ${sanser_speexdsp_SOURCE_DIR}/libspeexdsp/preprocess.c
  ${sanser_speexdsp_SOURCE_DIR}/libspeexdsp/filterbank.c
  ${sanser_speexdsp_SOURCE_DIR}/libspeexdsp/fftwrap.c
  ${sanser_speexdsp_SOURCE_DIR}/libspeexdsp/kiss_fft.c
  ${sanser_speexdsp_SOURCE_DIR}/libspeexdsp/kiss_fftr.c)
target_compile_definitions(sanser-speex-echo PRIVATE FLOATING_POINT USE_KISS_FFT EXPORT=)
target_include_directories(sanser-speex-echo PUBLIC
  ${sanser_speexdsp_SOURCE_DIR}/include ${_speex_generated})
add_library(sanser-voice-echo STATIC ${CMAKE_CURRENT_LIST_DIR}/voice_echo.cpp)
target_compile_features(sanser-voice-echo PUBLIC cxx_std_20)
target_include_directories(sanser-voice-echo PUBLIC ${CMAKE_CURRENT_LIST_DIR})
target_link_libraries(sanser-voice-echo PUBLIC sanser-speex-echo)
