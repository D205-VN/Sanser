#include "bmp_writer.h"

#include <fstream>
#include <stdexcept>

namespace {

template <typename T>
void writeValue(std::ofstream& out, T value) {
  out.write(reinterpret_cast<const char*>(&value), sizeof(T));
}

} // namespace

void writeBmp(const std::filesystem::path& path, const FrameBgra& frame) {
  if (frame.pixels.empty() || frame.width == 0 || frame.height == 0) {
    throw std::runtime_error("Cannot write an empty frame.");
  }

  std::ofstream out(path, std::ios::binary);
  if (!out) {
    throw std::runtime_error("Could not open BMP output file.");
  }

  const std::uint32_t headerSize = 14 + 40;
  const std::uint32_t imageSize = frame.stride * frame.height;
  const std::uint32_t fileSize = headerSize + imageSize;

  out.put('B');
  out.put('M');
  writeValue<std::uint32_t>(out, fileSize);
  writeValue<std::uint16_t>(out, 0);
  writeValue<std::uint16_t>(out, 0);
  writeValue<std::uint32_t>(out, headerSize);

  writeValue<std::uint32_t>(out, 40);
  writeValue<std::int32_t>(out, static_cast<std::int32_t>(frame.width));
  writeValue<std::int32_t>(out, -static_cast<std::int32_t>(frame.height));
  writeValue<std::uint16_t>(out, 1);
  writeValue<std::uint16_t>(out, 32);
  writeValue<std::uint32_t>(out, 0);
  writeValue<std::uint32_t>(out, imageSize);
  writeValue<std::int32_t>(out, 2835);
  writeValue<std::int32_t>(out, 2835);
  writeValue<std::uint32_t>(out, 0);
  writeValue<std::uint32_t>(out, 0);

  out.write(reinterpret_cast<const char*>(frame.pixels.data()), frame.pixels.size());
}
