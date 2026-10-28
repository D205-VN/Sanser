# Wired headphone output recovery after 2.2.24

The user reports crackling followed by silence while headphones are attached,
with sound returning after unplugging them. The initial description said USB;
the user corrected this to a **3.5 mm jack** and confirmed both machines run
2.2.24. The user subsequently confirmed the jack is on the **Mac controlling
Windows**, so this investigation targets the legacy Mac client playback path.
The filename preserves the initial investigation link; the reported hardware
is not USB.

## Current evidence

The latest local Mac client report is 2.2.24 over Direct. Its inspected samples
show approximately 200 authenticated audio datagrams/second, decoded/output
signal counters advancing, no codec errors and no output API error. The
cumulative output route-change/recovery counter reached 20 during the session.
No device IDs, names, speech or payloads are retained in this investigation.

These counters do **not** establish audible output from the headphone endpoint.
They also do not prove that the observed route changes were unintended; the
user may have been changing outputs while testing. The exact silent interval
has not been physically reproduced.

A read-only Core Audio snapshot identified the built-in headphone data source,
48 kHz stereo output with 32-bit samples, and a 512-frame device buffer
(approximately 10.7 ms). The optional jack-connected property was unsupported;
this does not mean that the jack was disconnected. Sanser supplies 5 ms PCM
blocks and keeps a bounded 20 ms hardware queue. The different block sizes
alone do not establish a fault and have not been changed. No device settings,
default outputs, capture or microphone permissions were changed for inspection.

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
4. Returned buffers and `IsRunning` did not establish that the output device's
   sample clock was advancing. A frozen clock could remain invisible even when
   the application continued receiving completion callbacks.

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
- Query `AudioQueueDeviceGetCurrentTime` with the running-state check. If valid
  sample time does not advance for 250 ms while audio is submitted, schedule
  output recreation with the existing 250 ms fault cooldown. Reset the baseline
  when recreating output or when clock queries are unsupported/invalid. Report
  `outputClockStalls` separately from API errors; do not invent an OSStatus.

## Validation

The device-free production AudioQueue adapter now exercises:

- Alternating valid/failed device metadata reads and fractional nominal-rate noise:
  no additional route recreations.
- A converter error with enqueue success and `IsRunning=true`: the error is
  detected, reported and the queue is replaced after the bounded cooldown.
- A delayed callback after failure: it cannot postpone the recovery deadline.
- Continued synthetic playback beyond forty seconds, including real route/rate
  changes, short continuity ramps and earlier stopped/stalled-output scenarios.
- Numeric-only connection-report filtering for the new converter counter.
- A frozen device clock with successful enqueues and completion callbacks:
  exactly one recovery on the same headphone route, without disconnecting.
- Unsupported clock queries and timestamps without valid sample time:
  no false clock-stall recovery.

The production client and latency-test executable compile locally. Targeted
client latency/media recovery tests pass without opening an audio device.
These changes are subsequent to the published 2.2.24 installer and do not change
an installed app automatically. A physical plug/unplug test on the Mac is still
required. Whether other local Mac audio remains audible during the fault is
not yet confirmed. A progressing hardware clock alone does not prove audible
sound, so this watchdog cannot detect every possible headphone failure.

## Platform references

Apple documents the converter error separately from the running state in
[AudioQueue properties](https://developer.apple.com/documentation/audiotoolbox/kaudioqueueproperty_currentdevice).
The local SDK `AudioQueue.h` also declares the converter error as a `UInt32`
property. No private APIs or device reconfiguration are used.
Apple's SDK also specifies that `AudioQueueDeviceGetCurrentTime` may provide
only host time when the device is stopped. Such timestamps are ignored by the
sample-clock watchdog rather than classified as a frozen sample clock.
