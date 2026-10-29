"""Bounded, reusable byte transformations shared by adapters and verification.

This module never chooses a script, dialogue row or save slot. Generated code
can call the public decode/view functions; only fixed host code launches tools.
"""
import base64
import bz2
import functools
import runpy
import json
import lzma
import os
import pathlib
import subprocess
import sys
import zlib

MAX_BYTES = 32 * 1024 * 1024
_SWAP_ADJACENT_BITS = bytes(((value & 0xAA) >> 1) | ((value & 0x55) << 1)
                            for value in range(256))


def _integer(data, proof):
    if not isinstance(proof, dict):
        raise ValueError('Decoded size needs native integer byte evidence')
    offset, width = proof.get('offset'), proof.get('width', 4)
    endian = proof.get('endian', 'little')
    if (type(offset) is not int or type(width) is not int or width not in (1, 2, 3, 4, 8)
            or endian not in ('little', 'big') or not 0 <= offset <= len(data) - width):
        raise ValueError('Decoded size field is outside its byte view')
    return int.from_bytes(data[offset:offset + width], endian)


def _region(data, offset, length):
    if (type(offset) is not int or type(length) is not int
            or not 0 <= offset <= len(data) or not 0 <= length <= len(data) - offset):
        raise ValueError('Byte-view slice is outside the source')
    return data[offset:offset + length]


def _erisa_assembly():
    # Both development and packaged layouts use the same bundled tool. No
    # adapter-controlled path, assembly, command or script is accepted.
    relative = pathlib.Path('Galgear-GalgameReverse-Skill/GalgameResourceExtract/tools/GARbro-cli/ArcFormats.dll')
    roots = [pathlib.Path(sys.executable).resolve().parent.parent.parent]
    source = pathlib.Path(__file__).resolve()
    if len(source.parents) > 4:
        roots.append(source.parents[4])
    for root in roots:
        candidate = root / relative
        if candidate.is_file():
            return candidate
    raise ValueError('Codec unavailable: erisa_nemesis needs the bundled resource tools')


def _erisa(data, expected_size):
    if os.name != 'nt':
        raise ValueError('Codec unavailable: erisa_nemesis requires the Windows resource runtime')
    if expected_size is None:
        raise ValueError('erisa_nemesis requires a native expected_size')
    script = pathlib.Path(__file__).with_name('decode-erisa.ps1')
    request = json.dumps(dict(data=base64.b64encode(data).decode('ascii'), size=expected_size)).encode('ascii')
    command = ['powershell.exe', '-NoLogo', '-NoProfile', '-NonInteractive',
               '-ExecutionPolicy', 'Bypass', '-File', str(script), '-Assembly', str(_erisa_assembly())]
    try:
        result = subprocess.run(command, input=request, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, timeout=20, creationflags=0x08000000)
    except subprocess.TimeoutExpired as error:
        raise ValueError('erisa_nemesis exceeded its decoder time limit') from error
    if result.returncode:
        raise ValueError('Invalid or unsupported erisa_nemesis stream')
    if len(result.stdout) > (MAX_BYTES + 2) // 3 * 4 + 4:
        raise ValueError('Decoded evidence exceeds its byte limit')
    try:
        return base64.b64decode(result.stdout.strip(), validate=True)
    except ValueError as error:
        raise ValueError('Invalid erisa_nemesis decoder response') from error


@functools.lru_cache(maxsize=2)
def _mdf(data):
    """Strict ordinary MDF/zlib frame only; no MT seed or encrypted shell."""
    if len(data) < 8 or data[:4] != b'mdf\0':
        raise ValueError('MDF needs a complete standard mdf header')
    expected = int.from_bytes(data[4:8], 'little')
    if expected > MAX_BYTES:
        raise ValueError('MDF native output size exceeds its byte budget')
    return decode('zlib', data[8:], expected)


def _freemote_assembly():
    relative = pathlib.Path('Galgear-GalgameReverse-Skill/GalgameResourceExtract/tools/freemote/FreeMote.dll')
    roots = [pathlib.Path(sys.executable).resolve().parent.parent.parent]
    source = pathlib.Path(__file__).resolve()
    if len(source.parents) > 4:
        roots.append(source.parents[4])
    for root in roots:
        candidate = root / relative
        if candidate.is_file():
            return candidate
    raise ValueError('Codec unavailable: psb_decrypt needs the bundled resource tools')


@functools.lru_cache(maxsize=2)
def _psb_decrypt(data, key):
    if os.name != 'nt':
        raise ValueError('Codec unavailable: psb_decrypt requires the Windows resource runtime')
    if (len(data) < 40 or data[:4] != b'PSB\0'
            or int.from_bytes(data[4:6], 'little') not in (2, 3)
            or int.from_bytes(data[6:8], 'little') not in (0, 1)):
        raise ValueError('PSB decryption requires a complete native version2/3 input')
    validator = runpy.run_path(str(pathlib.Path(__file__).with_name('typed_evidence.py')))
    try:
        validator['_parse_psb'](data)
    except ValueError:
        pass
    else:
        raise ValueError('PSB input is already complete plaintext; native Auto decryption is forbidden')
    script = pathlib.Path(__file__).with_name('decrypt-psb.ps1')
    request = json.dumps(dict(data=base64.b64encode(data).decode('ascii'), key=key)).encode('ascii')
    command = ['powershell.exe', '-NoLogo', '-NoProfile', '-NonInteractive',
               '-ExecutionPolicy', 'Bypass', '-File', str(script),
               '-ToolRoot', str(_freemote_assembly().parent)]
    try:
        completed = subprocess.run(command, input=request, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, timeout=20, creationflags=0x08000000)
    except subprocess.TimeoutExpired as error:
        raise ValueError('PSB native decryption exceeded its bounded runtime') from error
    if completed.returncode or len(completed.stdout) > (MAX_BYTES+2)//3*4+4:
        raise ValueError('Invalid or unsupported native PSB decryption')
    try:
        result = base64.b64decode(completed.stdout.strip(), validate=True)
    except ValueError as error:
        raise ValueError('Invalid native PSB decoder response') from error
    if len(result) != len(data) or result[:6] != data[:6] or result[6:8] != b'\0\0':
        raise ValueError('Native PSB decryption must preserve its complete binary and clear flags')
    if result[8:] == data[8:]:
        raise ValueError('Native PSB decryption did not change any encrypted content; flag-only normalization is forbidden')
    # Fixed validation of the ENTIRE result, including resource references.
    # This uses no adapter-supplied code or field selector and issues no role.
    validator['_parse_psb'](result)
    return result


def _tjs_text_stream(data):
    """Read only the fixed public KiriKiri text-stream wrapper; no field semantics."""
    data = bytes(data)
    if data.startswith(b'\xff\xfe'):
        body, mode = data[2:], None
    else:
        if (len(data) < 5 or data[:2] != b'\xfe\xfe'
                or data[2] not in (0, 1, 2) or data[3:5] != b'\xff\xfe'):
            raise ValueError('Invalid or unsupported TJS text-stream header/BOM')
        mode = data[2]
        if mode == 2:
            if len(data) < 21:
                raise ValueError('Truncated TJS text-stream size header')
            packed = int.from_bytes(data[5:13], 'little')
            unpacked = int.from_bytes(data[13:21], 'little')
            if (packed > MAX_BYTES or unpacked > MAX_BYTES or unpacked % 2
                    or packed != len(data) - 21):
                raise ValueError('Invalid TJS text-stream declared byte sizes')
            return decode('zlib', data[21:], unpacked)
        body = data[5:]
    if len(body) % 2:
        raise ValueError('TJS text stream needs complete UTF-16LE code units')
    if mode == 1:
        return body.translate(_SWAP_ADJACENT_BITS)
    if mode == 0:
        # Public krkrz TextStream.cpp reader semantics, including unchanged
        # controls. This legacy read transform is not a general-purpose encoder.
        output = bytearray(len(body))
        for index in range(0, len(body), 2):
            unit = body[index] | body[index + 1] << 8
            if unit >= 0x20:
                unit ^= ((unit & 0xfe) << 8) ^ 1
            output[index], output[index + 1] = unit & 255, unit >> 8
        return bytes(output)
    return body


def decode(codec, data, expected_size=None, *, key=None):
    """Decode bytes with fixed limits; exact sizes are optional except for ERISA."""
    if not isinstance(data, (bytes, bytearray)) or len(data) > MAX_BYTES:
        raise ValueError('Compressed evidence exceeds its input byte limit')
    if expected_size is not None and (type(expected_size) is not int or not 0 <= expected_size <= MAX_BYTES):
        raise ValueError('Invalid decoded evidence size')
    if codec == 'psb_decrypt':
        if type(key) is not int or not 0 <= key <= 0xffffffff:
            raise ValueError('PSB decryption requires one explicit unsigned32 key')
    elif key is not None:
        raise ValueError('This codec accepts no key parameter')
    maximum = MAX_BYTES if expected_size is None else expected_size
    try:
        if codec == 'mdf':
            result, decoder = _mdf(bytes(data)), None
        elif codec == 'psb_decrypt':
            result, decoder = _psb_decrypt(bytes(data), key), None
        elif codec == 'tjs_text_stream':
            result, decoder = _tjs_text_stream(data), None
        elif codec in ('zlib', 'deflate'):
            decoder = zlib.decompressobj(15 if codec == 'zlib' else -15)
            result = decoder.decompress(data, maximum + 1)
        elif codec == 'bz2':
            decoder = bz2.BZ2Decompressor()
            result = decoder.decompress(data, max_length=maximum + 1)
        elif codec == 'lzma':
            decoder = lzma.LZMADecompressor(memlimit=128 * 1024 * 1024)
            result = decoder.decompress(data, max_length=maximum + 1)
        elif codec == 'erisa_nemesis':
            result, decoder = _erisa(data, expected_size), None
        else:
            raise ValueError('Unsupported byte codec: ' + str(codec)[:64])
    except (zlib.error, OSError, EOFError, lzma.LZMAError) as error:
        raise ValueError('Invalid compressed evidence for codec ' + str(codec)[:64]) from error
    if (len(result) > maximum or expected_size is not None and len(result) != expected_size
            or decoder is not None and (not decoder.eof or decoder.unused_data)):
        raise ValueError('Compressed evidence is oversized, incomplete or has trailing data')
    return result


def view(data, steps):
    """Replay an adapter's declared transforms against the original input."""
    if not isinstance(data, (bytes, bytearray)) or len(data) > MAX_BYTES:
        raise ValueError('Byte-view evidence exceeds its input byte limit')
    data = bytes(data)
    if not isinstance(steps, list) or len(steps) > 8:
        raise ValueError('Invalid byte-view transform list')
    for step in steps:
        if not isinstance(step, dict):
            raise ValueError('Invalid byte-view transform')
        op = step.get('op')
        if op == 'slice':
            data = _region(data, step.get('offset'), step.get('length'))
        elif op in ('xor', 'xor_range'):
            allowed = {'op', 'key'} if op == 'xor' else {'op', 'key', 'offset', 'length'}
            if set(step) != allowed:
                raise ValueError('XOR evidence needs its exact bounded transform fields')
            try:
                key = bytes.fromhex(step.get('key', ''))
            except (ValueError, TypeError) as error:
                raise ValueError('Invalid XOR evidence key') from error
            if not 1 <= len(key) <= 32:
                raise ValueError('XOR evidence key must be 1 to 32 bytes')
            if op == 'xor_range':
                offset, length = step['offset'], step['length']
                region = _region(data, offset, length)
                if not length:
                    raise ValueError('XOR evidence range must be nonempty')
                transformed = bytes(value ^ key[index % len(key)] for index, value in enumerate(region))
                data = data[:offset] + transformed + data[offset+length:]
            else:
                data = bytes(value ^ key[index % len(key)] for index, value in enumerate(data))
        elif op == 'mdf':
            if set(step) != {'op'}:
                raise ValueError('Ordinary MDF accepts no key, offset or shell parameters')
            data = decode('mdf', data)
        elif op == 'psb_decrypt':
            if set(step) != {'op', 'key'}:
                raise ValueError('PSB decryption accepts only its explicit unsigned32 key')
            data = decode('psb_decrypt', data, key=step['key'])
        elif op == 'tjs_text_stream':
            if set(step) != {'op'}:
                raise ValueError('TJS text-stream view accepts no keys, paths or layout parameters')
            data = decode('tjs_text_stream', data)
        elif op == 'swap_adjacent_bits':
            # A fixed byte permutation, also used by KiriKiri text-stream mode 1:
            # krkrz/krkrz fd5c4baa base/TextStream.cpp, Read(), CryptMode == 1.
            # Header/container selection remains the adapter's responsibility.
            if set(step) != {'op'}:
                raise ValueError('Adjacent-bit permutation has no model-supplied parameters')
            data = data.translate(_SWAP_ADJACENT_BITS)
        elif op in ('zlib', 'deflate', 'bz2', 'lzma', 'erisa_nemesis'):
            size = _integer(data, step['size_from']) if 'size_from' in step else None
            if op == 'erisa_nemesis' and size is None:
                raise ValueError('erisa_nemesis view requires size_from in the original byte view')
            compressed = _region(data, step['offset'], step['length']) if 'offset' in step else data
            data = decode(op, compressed, size)
        else:
            raise ValueError('Unsupported byte-view transform')
    return data


def capabilities():
    return dict(version=3, codecs=['zlib', 'deflate', 'bz2', 'lzma', 'erisa_nemesis', 'tjs_text_stream', 'mdf', 'psb_decrypt'],
                transforms=['slice', 'xor', 'xor_range', 'swap_adjacent_bits', 'tjs_text_stream', 'mdf', 'psb_decrypt'], max_bytes=MAX_BYTES, max_steps=8,
                mdf='Ordinary complete MDF signature + u32le output size + exact bounded zlib; no encrypted shell, seed, offset or unknown fields.',
                psb_decrypt='decode("psb_decrypt",data,key=<explicit u32>) or {op:"psb_decrypt",key:<u32>}. '
                    'Fixed bundled FreeMote v4.9.0 guarded native Auto+Decrypt only. Plaintext/unknown keys fail; '
                    'whole binary length, strict v2/v3 zero flags and complete typed root are required. '
                    'No mode/position/path/CLR/offset or key search. No current/resume role is issued.',
                tjs_text_stream=dict(input='FFFE plaintext or FEFE mode0/1/2 + FFFE', output='UTF-16LE body without the outer BOM',
                    framing='Fixed native mode2 u64le sizes and complete zlib EOF; no external mode/key/path. BMP requires an explicit prior slice.',
                    scope='Byte decoding only; never slot, current-node or resume-role attestation',
                    reference='krkrz/krkrz fd5c4baa base/TextStream.cpp'),
                xor_range='Exact {op:"xor_range",offset,length,key:<1..32 hex bytes>}; bounded repeating XOR only. '
                          'This transform is not an archive-filter attestation or checksum proof.',
                erisa_size='size_from:{offset,width:4,endian:"little"} in the current input view')
