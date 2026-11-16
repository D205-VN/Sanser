include_guard(GLOBAL)
include(FetchContent)
if(POLICY CMP0135)
  cmake_policy(SET CMP0135 NEW)
endif()
# RNNoise 0.2 (Xiph, BSD-3-Clause): recurrent-network noise suppression for
# group voice. The release tarball bundles its trained weights, so nothing is
# downloaded at runtime. Portable C only: no x86 runtime CPU dispatch.
enable_language(C)
FetchContent_Declare(sanser_rnnoise
  URL https://github.com/xiph/rnnoise/releases/download/v0.2/rnnoise-0.2.tar.gz
  URL_HASH SHA256=90fce4b00b9ff24c08dbfe31b82ffd43bae383d85c5535676d28b0a2b11c0d37)
FetchContent_MakeAvailable(sanser_rnnoise)
set(_rnnoise ${sanser_rnnoise_SOURCE_DIR}/src)
# The 0.2 tarball's vec.h includes os_support.h, which the release omits.
# Provide only the three copy/clear macros it uses.
set(_rnnoise_shim "${CMAKE_CURRENT_BINARY_DIR}/rnnoise-shim")
file(MAKE_DIRECTORY "${_rnnoise_shim}")
file(WRITE "${_rnnoise_shim}/os_support.h"
  "#ifndef SANSER_RNNOISE_OS_SUPPORT_H\n#define SANSER_RNNOISE_OS_SUPPORT_H\n#include <string.h>\n"
  "#define OPUS_CLEAR(dst,n) (memset((dst),0,(n)*sizeof(*(dst))))\n"
  "#define OPUS_COPY(dst,src,n) (memcpy((dst),(src),(n)*sizeof(*(dst))))\n"
  "#define OPUS_MOVE(dst,src,n) (memmove((dst),(src),(n)*sizeof(*(dst))))\n#endif\n")
add_library(sanser-rnnoise STATIC
  ${_rnnoise}/denoise.c ${_rnnoise}/rnn.c ${_rnnoise}/pitch.c ${_rnnoise}/kiss_fft.c
  ${_rnnoise}/celt_lpc.c ${_rnnoise}/nnet.c ${_rnnoise}/nnet_default.c
  ${_rnnoise}/parse_lpcnet_weights.c ${_rnnoise}/rnnoise_data.c ${_rnnoise}/rnnoise_tables.c)
target_include_directories(sanser-rnnoise PUBLIC ${sanser_rnnoise_SOURCE_DIR}/include PRIVATE ${_rnnoise_shim} ${_rnnoise})
target_compile_definitions(sanser-rnnoise PRIVATE RNNOISE_BUILD)
if(MSVC)
  target_compile_definitions(sanser-rnnoise PRIVATE restrict=__restrict _USE_MATH_DEFINES)
  target_compile_options(sanser-rnnoise PRIVATE /wd4244 /wd4305 /wd4267)
else()
  target_compile_options(sanser-rnnoise PRIVATE -w)
endif()
if(NOT MSVC)
  target_link_libraries(sanser-rnnoise PRIVATE m)
endif()
