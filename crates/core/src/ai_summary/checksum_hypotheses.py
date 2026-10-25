"""Bounded checksum hypotheses, never a decoder or verification receipt.

The Adler-32 sums and initialization follow RFC 1950 section 2.2:
https://www.rfc-editor.org/rfc/rfc1950.html#section-2.2
For zero-based byte index i in n bytes, changing that byte by delta changes
s1 by delta and s2 by (n-i)*delta, modulo 65521. Scanning prefix endpoints
therefore needs linear time and bounded result storage, without making n
decoded copies. No format, key, checksum source, or semantic role is inferred.
"""
import zlib

MAX_BYTES = 32 * 1024 * 1024
MAX_CANDIDATES = 4096
_MODULUS = 65521


def adler32_xor_prefix_lengths(data, key, target_checksum, max_candidates=64):
    """Enumerate all XOR-prefix lengths satisfying a caller's Adler-32 number.

    For each L in [0,len(data)], the hypothetical input is data with exactly
    its first L bytes XORed by the supplied nonzero byte key. The full match
    count is computed even after the ascending length list reaches its cap.
    This function returns numbers only. A collision is not decoder selection,
    native checksum provenance, archive identity, or a script/save attestation.
    """
    if type(data) not in (bytes, bytearray):
        raise TypeError('Checksum hypotheses require bytes or bytearray input')
    if len(data) > MAX_BYTES:
        raise ValueError('Checksum hypotheses input exceeds 32 MiB')
    if type(key) is not int or not 1 <= key <= 255:
        raise ValueError('Checksum hypotheses require an explicit nonzero byte key')
    if type(target_checksum) is not int or not 0 <= target_checksum <= 0xffffffff:
        raise ValueError('Checksum hypotheses require an explicit unsigned 32-bit target')
    if type(max_candidates) is not int or not 0 <= max_candidates <= MAX_CANDIDATES:
        raise ValueError('Checksum hypotheses candidate cap must be an integer from 0 to 4096')

    size = len(data)
    original = zlib.adler32(data) & 0xffffffff
    s1, s2 = original & 0xffff, original >> 16
    wanted_s1, wanted_s2 = target_checksum & 0xffff, target_checksum >> 16
    lengths = []
    matches = 0
    if s1 == wanted_s1 and s2 == wanted_s2:
        matches = 1
        if max_candidates:
            lengths.append(0)
    for index, value in enumerate(data):
        delta = (value ^ key) - value
        s1 = (s1 + delta) % _MODULUS
        s2 = (s2 + (size - index) * delta) % _MODULUS
        if s1 == wanted_s1 and s2 == wanted_s2:
            matches += 1
            if len(lengths) < max_candidates:
                lengths.append(index + 1)
    return dict(match_count=matches, lengths=lengths,
                truncated=matches > len(lengths), checked_prefixes=size + 1)


def capabilities():
    return dict(
        function='adler32_xor_prefix_lengths(data,key,target_checksum,max_candidates=64)',
        input='bytes or bytearray <=32 MiB; explicit integer key 1..255 and u32 target_checksum',
        output='All prefix endpoints including 0: complete match_count, ascending lengths, truncated, checked_prefixes',
        max_candidates=MAX_CANDIDATES,
        complexity='O(input bytes) time; O(max_candidates) result storage',
        scope='Numerical hypotheses only. Never infers a key, checksum source, decoder, offset, format or node role. '
              'Caller-supplied checksum is not native archive evidence. The model must choose any member_filter; '
              'native metadata, full-member checksum, complete typed data and semantic-role verification still apply.',
        reference='RFC 1950 section 2.2 (Adler-32)')
