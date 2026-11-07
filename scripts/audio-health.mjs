// Numeric export projection. Never copies arbitrary keys, speech or identities.
export const audioCounterNames = [
  'voiceCapturedFrames', 'voiceDecodedPackets', 'voiceConcealedPackets', 'voiceFecAttempts',
  'voiceDroppedPackets', 'voiceLatePackets', 'voiceInvalidPackets', 'voiceRawClippedFrames',
  'voiceCleanClippedFrames', 'voiceLimitedFrames', 'voiceCaptureQueueDrops', 'voiceIncomingQueueDrops',
  'voiceStaleCaptureBlocks', 'voicePacketsSent', 'voicePcmUnderflows', 'voicePcmCapacityDrops',
  'voicePcmStaleDrops', 'voicePcmReprimes', 'voiceAecBlocks', 'voiceAecMissingReference',
  'voiceAecDryBlocks', 'voiceAecResets', 'systemDecodedPackets', 'systemConcealedPackets',
  'systemDecodeErrors', 'systemPcmUnderflows', 'systemPcmCapacityDrops', 'systemPcmStaleDrops',
  'systemPcmReprimes', 'systemPcmResets', 'systemJitterUnderflows', 'systemJitterQueueDrops',
  'systemOutputErrors', 'systemOutputQueueDrops', 'systemOutputRecoveries', 'systemOutputRouteChanges',
  'systemConverterErrors', 'systemClockStalls'
];
const integer = (n) => Number.isSafeInteger(n) && n >= 0;
export function summarizeAudio(report) {
  const counters = report.performance?.audio?.counters ?? {};
  return Object.fromEntries(audioCounterNames.map((name) => {
    const v = counters[name];
    if (!v || !['native-voice', 'legacy-system-audio', 'snv2-system-audio'].includes(v.source) ||
      ![v.latest, v.observedIncrease, v.observedDurationMs, v.resets, v.samples].every(integer) || v.samples < 1 ||
      v.observedDurationMs > (v.samples - 1) * 5000 ||
      (v.observedDurationMs === 0 && v.observedIncrease !== 0)) return [name, null];
    return [name, { latest: v.latest, observedIncrease: v.observedIncrease, observedDurationMs: v.observedDurationMs,
      resets: v.resets, samples: v.samples, source: v.source,
      eventsPerSecond: v.observedDurationMs ? v.observedIncrease * 1000 / v.observedDurationMs : null,
      scope: 'observed-counter-intervals' }];
  }));
}
