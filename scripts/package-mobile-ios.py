#!/usr/bin/env python3
"""Package a release iphoneos app for local re-signing, never a Simulator app."""
import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import struct
import zipfile


def require_device_binary(path):
    require_device_image(path.read_bytes(), str(path))


def require_device_image(data, path, allow_fat=True):
    # Flutter AOT may use a universal container even with only an ARM64 slice.
    if allow_fat and data[:4] in (b'\xca\xfe\xba\xbe', b'\xca\xfe\xba\xbf'):
        if len(data) < 8:
            raise ValueError(f'Truncated universal header: {path}')
        count = struct.unpack_from('>I', data, 4)[0]
        wide = data[:4] == b'\xca\xfe\xba\xbf'
        size = 32 if wide else 20
        if not 1 <= count <= 32 or 8 + count * size > len(data):
            raise ValueError(f'Invalid universal header: {path}')
        for index in range(count):
            base = 8 + index * size
            cpu = struct.unpack_from('>I', data, base)[0]
            start, length = struct.unpack_from('>QQ' if wide else '>II', data, base + 8)
            if cpu != 0x0100000C or start < 8 + count * size or start + length > len(data):
                raise ValueError(f'Invalid iPhone slice: {path}')
            require_device_image(data[start:start + length], f'{path} slice {index}', False)
        return
    # Fail closed on unknown layouts rather than publishing a renamed Simulator ZIP.
    if len(data) < 32 or data[:4] != b'\xcf\xfa\xed\xfe':
        raise ValueError(f'Expected a thin ARM64 Mach-O: {path}')
    _, cpu, _, _, commands, command_bytes, _, _ = struct.unpack_from('<8I', data)
    if cpu != 0x0100000C or 32 + command_bytes > len(data):
        raise ValueError(f'Invalid ARM64 Mach-O: {path}')
    offset, device = 32, False
    for _ in range(commands):
        if offset + 8 > 32 + command_bytes:
            raise ValueError(f'Truncated load commands: {path}')
        command, size = struct.unpack_from('<II', data, offset)
        if size < 8 or offset + size > 32 + command_bytes:
            raise ValueError(f'Invalid load command: {path}')
        if command == 0x32:  # LC_BUILD_VERSION; platform 2 = physical iOS.
            if size < 24 or struct.unpack_from('<I', data, offset + 8)[0] != 2:
                raise ValueError(f'Non-iPhone platform in {path}')
            device = True
        elif command == 0x25 and size >= 16:  # LC_VERSION_MIN_IPHONEOS
            device = True
        if command == 0x2C and size >= 24:  # LC_ENCRYPTION_INFO_64
            if struct.unpack_from('<I', data, offset + 16)[0]:
                raise ValueError(f'Encrypted executable cannot be re-signed: {path}')
        offset += size
    if not device or offset != 32 + command_bytes:
        raise ValueError(f'Missing valid iPhone platform commands: {path}')


def package(app, output):
    app = app.resolve(strict=True)
    info = plistlib.loads((app / 'Info.plist').read_bytes())
    if info.get('CFBundleSupportedPlatforms') != ['iPhoneOS']:
        raise ValueError('Expected iPhoneOS; Simulator apps cannot install on iPhone')
    if info.get('CFBundleIdentifier') != 'com.sanser.mobile':
        raise ValueError('Expected the Sanser mobile bundle identifier')
    if (app / 'embedded.mobileprovision').exists():
        raise ValueError('Use --no-codesign; do not distribute a personal profile')
    if (app / 'Frameworks/App.framework/flutter_assets/kernel_blob.bin').exists():
        raise ValueError('Use a release/AOT build, not a debug/JIT build')
    files = sorted(p for p in app.rglob('*') if p.is_file() or p.is_symlink())
    binaries = []
    require_device_binary(app / info['CFBundleExecutable'])
    for path in files:
        if path.is_symlink():
            raise ValueError(f'Unexpected symlink in device app: {path}')
        with path.open('rb') as stream:
            magic = stream.read(4)
        if magic in (b'\xcf\xfa\xed\xfe', b'\xca\xfe\xba\xbe', b'\xca\xfe\xba\xbf',
                     b'\xfe\xed\xfa\xcf'):
            require_device_binary(path)
            binaries.append(str(path.relative_to(app)))
    if 'Frameworks/App.framework/App' not in binaries:
        raise ValueError('Missing compiled Flutter AOT application')
    output = output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    # Exclusive creation prevents accidentally replacing a published artifact.
    with zipfile.ZipFile(output, 'x', zipfile.ZIP_DEFLATED) as archive:
        for path in files:
            archive.write(path, 'Payload/Sanser.app/' + str(path.relative_to(app)))
    with zipfile.ZipFile(output) as archive:
        if archive.testzip():
            raise ValueError('IPA ZIP integrity check failed')
    digest = hashlib.sha256(output.read_bytes()).hexdigest()
    output.with_suffix('.ipa.sha256').write_text(f'{digest}  {output.name}\n')
    report = dict(bundleIdentifier=info['CFBundleIdentifier'],
                  version=info['CFBundleShortVersionString'], build=info['CFBundleVersion'],
                  platform='iPhoneOS', minimumOS=info['MinimumOSVersion'],
                  signing='requires recipient re-signing; not an App Store/TestFlight build',
                  binaries=binaries, bytes=output.stat().st_size, sha256=digest)
    output.with_suffix('.ipa.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('app', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    try:
        package(args.app, args.output)
    except (ValueError, OSError, KeyError, plistlib.InvalidFileException) as error:
        parser.exit(1, f'{error}\n')
