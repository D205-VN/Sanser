#pragma once

void testCompletedVideoRepairDedup() {
  const auto token=gExpectedSessionToken;
  const auto generation=gMediaPeerGeneration.load();
  struct Restore {
    std::string token;std::uint64_t generation;
    ~Restore() { gExpectedSessionToken=token;gMediaPeerGeneration=generation; }
  } restore{token,generation};
  gExpectedSessionToken.clear();
  UdpVideoReassembler receiver;
  UdpVideoStats stats;
  UdpPeerLock peerLock("completed-frame-repair-test");
  const auto peer=resolveUdpAddress("127.0.0.1:55000");
  std::vector<std::uint8_t> bytes;
  std::uint64_t id=0,epoch=0;bool grace=false;
  const auto fragment=[&](std::uint64_t packet,std::uint16_t index,bool repair=false,std::uint16_t count=3) {
    UdpVideoFragmentHeader header;
    header.packetId=packet;header.packetSize=count;header.fragmentCount=count;
    header.fragmentIndex=index;header.fragmentOffset=index;header.payloadSize=1;header.flags=repair ? 4 : 0;
    std::vector<std::uint8_t> datagram(sizeof(header)+1,static_cast<std::uint8_t>(index));
    std::memcpy(datagram.data(),&header,sizeof(header));
    return receiver.push(datagram,bytes,stats,id,epoch,grace,peer,peerLock);
  };
  requireLatency(!fragment(1,0) && !fragment(1,2) && fragment(1,1,true),
    "A retransmitted missing fragment must complete the original assembly");
  requireLatency(bytes==std::vector<std::uint8_t>({0,1,2}),"Repaired video payload changed");
  stats.newNackPacketIds.clear();stats.nackPacketIds.clear();
  requireLatency(!fragment(1,2,true) && receiver.incompletePackets()==0,
    "Late repair fragment recreated an assembly for an already completed video frame");
  for(unsigned retry=0;retry<3;++retry) for(std::uint16_t i=0;i<3;++i)
    requireLatency(!fragment(1,i,true),"A full retransmit emitted the same completed video frame again");
  receiver.pollRepairs(stats,std::chrono::steady_clock::now()+std::chrono::milliseconds(100));
  requireLatency(stats.completedPackets==1 && stats.retransmitCompletedPackets==1 &&
    receiver.incompletePackets()==0 && stats.newNackPacketIds.empty() && stats.droppedAssemblies==0,
    "A completed-frame retransmit created false loss/NACK pressure");
  const auto retired=stats.completedDuplicateFragments;
  gExpectedSessionToken=std::string(48,'s');
  requireLatency(!fragment(1,0,true) && stats.authRejectedDatagrams==1 && stats.completedDuplicateFragments==retired,
    "Completion filtering bypassed session authentication or hid rejected traffic");
  gExpectedSessionToken.clear();
  // Keep completion memory bounded while retaining a stale-ID floor. A cache
  // eviction must not make an ancient authenticated repair useful again.
  for(std::uint64_t packet=2;packet<=700;++packet)
    requireLatency(fragment(packet,0,false,1),"New video frame was incorrectly retired");
  requireLatency(!fragment(1,0,true) && receiver.incompletePackets()==0,
    "A very old repair came back after completion-window eviction");
  requireLatency(fragment(702,0,false,1) && fragment(701,0,true,1),
    "Completion filtering must still allow a previously unseen reordered frame");
  gMediaPeerGeneration=generation+1;
  requireLatency(!fragment(1,0) && !fragment(1,1) && fragment(1,2),
    "A new media generation could not reuse its reset packet ID");
  requireLatency(!fragment(2,0) && !fragment(2,2), "Incomplete frame unexpectedly completed");
  const auto holes=receiver.missingRanges({2});
  requireLatency(holes.size()==1 && holes.front().packetId==2 && holes.front().first==1 && holes.front().last==1,
    "NACK did not describe the actual missing fragment in the production reassembler");
  requireLatency(fragment(2,1,true) && receiver.missingRanges({2}).empty(),
    "Completed packet retained selective repair hints");
  std::vector<std::uint64_t> ids;
  std::vector<sanser::video::FragmentRange> ranges;
  for(unsigned i=0;i<sanser::video::maxNackPackets;++i) ids.push_back(UINT64_MAX-i);
  for(unsigned i=0;i<sanser::video::maxNackRanges;++i) ranges.push_back({ids[i%ids.size()],65534,65534});
  const auto json=videoNackPayload("udp-loss",ids,ranges,UINT64_MAX);
  const auto envelope=makeSecureControlEnvelope(std::string(48,'s'),"client",UINT64_MAX,json);
  requireLatency(envelope.size()+1<=1200,"Authenticated fragment NACK exceeds the safe UDP payload budget");
  requireLatency(json.find("\"missingRanges\":[")!=std::string::npos &&
    json.find("\"packetIds\":[")!=std::string::npos,"NACK lost backwards-compatible packet IDs");
}
