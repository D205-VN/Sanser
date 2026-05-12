const fs = require('fs');

const file = process.argv[2];
if (!file) {
  console.error('Usage: node scripts/inspect-snv.js <capture_h264.snv>');
  process.exit(1);
}

const data = fs.readFileSync(file);
let offset = 0;
let packets = 0;
let keyframes = 0;
let first = null;
let last = null;

while (offset + 52 <= data.length) {
  const magic = data.toString('ascii', offset, offset + 4);
  if (magic !== 'SNV1') {
    throw new Error(`Invalid SNV1 magic at byte ${offset}: ${magic}`);
  }

  const headerSize = data.readUInt32LE(offset + 4);
  if (headerSize < 52 || offset + headerSize > data.length) {
    throw new Error(`Invalid header size ${headerSize} at byte ${offset}`);
  }

  const header = {
    codec: data.readUInt32LE(offset + 8),
    packetFormat: data.readUInt32LE(offset + 12),
    width: data.readUInt32LE(offset + 16),
    height: data.readUInt32LE(offset + 20),
    sequence: Number(data.readBigUInt64LE(offset + 24)),
    timestampMicros: Number(data.readBigUInt64LE(offset + 32)),
    durationMicros: data.readUInt32LE(offset + 40),
    flags: data.readUInt32LE(offset + 44),
    payloadSize: data.readUInt32LE(offset + 48),
  };

  const payloadOffset = offset + headerSize;
  const nextOffset = payloadOffset + header.payloadSize;
  if (nextOffset > data.length) {
    throw new Error(`Packet ${header.sequence} payload exceeds file length`);
  }

  if (header.flags & 1) keyframes += 1;
  if (!first) first = header;
  last = header;
  packets += 1;
  offset = nextOffset;
}

if (offset !== data.length) {
  throw new Error(`Trailing ${data.length - offset} byte(s) after last packet`);
}

console.log(JSON.stringify({
  file,
  bytes: data.length,
  packets,
  keyframes,
  first,
  last,
}, null, 2));
