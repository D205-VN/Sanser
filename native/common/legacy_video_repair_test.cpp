#include "legacy_video_repair.h"
#include <iostream>
#include <stdexcept>

static void require(bool condition, const char* message) {
  if (!condition) throw std::runtime_error(message);
}
int main() {
  using namespace sanser::video;
  std::vector<std::uint64_t> ids{71, 72};
  std::vector<std::uint64_t> flat{71, 198, 199, 71, 3, 4, 71, 4, 5};
  auto hints = decodeMissingRanges(flat, ids);
  RepairCursor repair(71, hints);
  require(repair.selective() && repair.first() == 3 && repair.chunkSize(6) == 3,
          "Overlapping holes were not sorted and merged");
  repair.advance(1, false);
  require(repair.first() == 4 && repair.chunkSize(6) == 2,
          "Partial socket send skipped missing fragments");
  repair.advance(2, false);
  require(repair.first() == 198 && repair.chunkSize(6) == 2,
          "Tail repair retransmitted the already received frame prefix");
  repair.advance(2, true);
  require(repair.complete(), "Tail repair did not terminate");
  RepairCursor interior(71, decodeMissingRanges(std::vector<std::uint64_t>{71, 150, 150}, ids));
  require(interior.first() == 150 && interior.chunkSize(24) == 1,
          "One missing fragment in a 200-fragment frame was expanded to a full repair");
  interior.advance(1, false);
  require(interior.complete(), "Interior repair incorrectly waited for the whole frame tail");
  for (const auto& invalid : std::vector<std::vector<std::uint64_t>>{
      {71, 1}, {71, 5, 4}, {73, 1, 2}, {71, 0, 65535}, {0, 1, 2},
      std::vector<std::uint64_t>((maxNackRanges + 1) * 3, 71)}) {
    require(decodeMissingRanges(invalid, ids).empty(), "Invalid/oversized hint was accepted");
  }
  RepairCursor legacy(72, hints);
  require(!legacy.selective() && legacy.first() == 0 && legacy.chunkSize(6) == 6,
          "Old peers or absent hints lost whole-frame fallback");
  legacy.advance(0, false);
  require(legacy.first() == 0 && !legacy.complete(), "Would-block consumed a fragment");
  legacy.advance(6, false);
  require(legacy.first() == 6 && !legacy.complete(), "Legacy cursor did not resume");
  legacy.advance(2, true);
  require(legacy.complete(), "Legacy tail did not complete");
  std::vector<std::uint8_t> received(200, 1);received[198] = received[199] = 0;
  std::vector<FragmentRange> ranges;
  require(appendMissingRanges(71, received, ranges) && ranges.size() == 1 &&
          ranges[0].first == 198 && ranges[0].last == 199, "Bitmap did not describe the missing tail");
  std::vector<std::uint8_t> fragmented(40);
  for (std::size_t i = 0; i < fragmented.size(); i += 2) fragmented[i] = 1;
  require(!appendMissingRanges(72, fragmented, ranges) && ranges.size() == 1 && ranges[0].packetId == 71,
          "Range cap serialized a partial hole list instead of preserving whole-frame fallback");
  std::cout << "Selective repair: tail/interior loss, partial sends, bounded hints and legacy fallback passed\n";
}
