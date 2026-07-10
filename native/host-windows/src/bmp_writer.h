#pragma once

#include "desktop_duplication.h"

#include <filesystem>

void writeBmp(const std::filesystem::path& path, const FrameBgra& frame);
