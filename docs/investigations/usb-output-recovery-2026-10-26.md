# USB output recovery after 2.2.24

The user reports crackling followed by silence while a USB cable/device is
attached, with sound returning after unplugging it. Device type, attachment
machine and the other machine's version have been requested and are pending.
Do not assume the USB device is a Windows capture endpoint or a Mac headset.

## Current evidence

The latest local Mac client report is 2.2.24 over Direct. Its inspected samples
show approximately 200 authenticated audio datagrams/second, decoded/output
signal counters advancing, no codec errors and no output API error. The
cumulative output route-change/recovery counter reached 20 during the session.
No device IDs, names, speech or payloads are retained in this investigation.

These counters do **not** establish audible output from the USB endpoint.
They also do not prove that the observed route changes were unintended; the
user may have been changing outputs while testing. The affected device and
exact silent interval have not been physically reproduced.

## Confirmed gaps in the Mac playback implementation

1. AudioQueue enqueue success and `IsRunning` did not inspect the queue's
   separate converter-error property. A nonzero converter error could therefore
   remain invisible while the application continued submitting valid PCM.
2. Optional device-property read failures were represented as source/rate zero.
   Comparing those values with the last successful read caused unnecessary
   route recreation; tiny floating-point nominal-rate variations also compared
   unequal. Neither establishes a real route change.
3. The retry cooldown for a known output error depended on the most recent
   returned-buffer callback. Callback activity should not extend that cooldown.

## Source changes

- Query `kAudioQueueProperty_ConverterError` alongside the running-state check.
  A supported, nonzero error enters bounded output recreation and retains the
  original OSStatus plus a numeric `outputConverterErrors` counter.
  An unsupported query is not itself classified as a converter failure.
- Keep validity for optional source/rate readings. Preserve the last known
  values on temporary failure and ignore nominal-rate changes within 1 Hz.
  Device changes and meaningful valid source/rate changes still recreate output.
- Base the retry deadline on the first observed fault, independently of later
  buffer callbacks. Recovery remains limited to the audio output path.

## Validation

The device-free production AudioQueue adapter now exercises:

- Alternating valid/failed USB metadata reads and fractional nominal-rate noise:
  no additional route recreations.
- A converter error with enqueue success and `IsRunning=true`: the error is
  detected, reported and the queue is replaced after the bounded cooldown.
- A delayed callback after failure: it cannot postpone the recovery deadline.
- Continued synthetic playback beyond forty seconds, including real route/rate
  changes, short continuity ramps and earlier stopped/stalled-output scenarios.
- Numeric-only connection-report filtering for the new converter counter.

The pending hardware clarification is still required before attributing the
user's USB crackle/silence to these cases. These changes are subsequent to the
published 2.2.24 installer and do not change an installed app automatically.

## Platform references

Apple documents the converter error separately from the running state in
[AudioQueue properties](https://developer.apple.com/documentation/audiotoolbox/kaudioqueueproperty_currentdevice).
The local SDK `AudioQueue.h` also declares the converter error as a `UInt32`
property. No private APIs or device reconfiguration are used.
