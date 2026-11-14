"""Fixed read-only archive routes; candidate detection never proves precedence."""
import fnmatch
import hashlib
import json
import pathlib
import re
import struct


BACKENDS = {
    'pfs': ('tools/pfs-rs/pfs-rs.exe', 'commands/pfs-rs.md'),
    'siglus': ('tools/siglus-ssu/siglus-ssu.exe', 'commands/siglus-ssu.md'),
    'garbro-mod': ('tools/GARbro-cli/GARbro.Console.exe', 'commands/garbro.md'),
}
MAX_SPECIALIZED_ARCHIVE = 256 * 1024 * 1024


def selection_error(category, message, **facts):
    """Keep resource discovery blockers distinct from parser/format failures."""
    error = ValueError(message)
    error.error_details = dict(category=category, **facts)
    return error


def mod_installed(skill):
    marker = skill / 'tools/GARbro-cli/provenance.json'
    try:
        return (marker.stat().st_size < 65536
                and json.loads(marker.read_text('utf-8')).get('distribution', '').startswith('GARbro-Mod-'))
    except (OSError, ValueError, AttributeError):
        return False


def equivalent_sources(paths, selected, cache):
    """Group byte-identical candidates; names alone never establish equivalence."""
    sizes = {}
    for path in paths:
        sizes.setdefault(path.stat().st_size, []).append(path)
    groups = []
    for candidates in sizes.values():
        if len(candidates) == 1:
            groups.append(candidates)
            continue
        hashes = {}
        for path in candidates:
            before = path.stat()
            stamp = (str(path), before.st_size, before.st_mtime_ns, before.st_ctime_ns)
            digest = cache.get(stamp)
            if digest is None:
                value = hashlib.sha256()
                with path.open('rb') as stream:
                    for chunk in iter(lambda: stream.read(1024 * 1024), b''):
                        value.update(chunk)
                after = path.stat()
                if (after.st_size, after.st_mtime_ns, after.st_ctime_ns) != stamp[1:]:
                    raise ValueError('Resource changed while checking duplicate content')
                digest = cache[stamp] = value.hexdigest()
            hashes.setdefault(digest, []).append(path)
        groups.extend(hashes.values())
    # Preserve the model-selected location when it is an equivalent copy.
    return [[selected] + [p for p in group if p != selected] if selected in group else group
            for group in groups]


def classify(path):
    with path.open('rb') as stream:
        head = stream.read(92)
    if head[:3] in (b'pf6', b'pf8'):
        return 'pfs'
    if len(head) == 92:
        h = struct.unpack('<23i', head)
        if (h[0] == 92 and 0 < h[14] == h[16] == h[18] == h[20] <= 20000
                and h[21] in (0, 1) and 92 <= h[19] <= path.stat().st_size
                and all(92 <= offset <= h[19] - 8 * h[20] for offset in (h[13], h[17]))
                and 92 <= h[15] < h[19]):
            return 'siglus'
    return 'garbro'


def scene_members(path):
    """Read only the bounded scene name and data indices, without decoding payloads."""
    with path.open('rb') as stream:
        h = struct.unpack('<23i', stream.read(92))
        size = path.stat().st_size
        count, base = h[20], h[19]
        if not 92 <= base <= min(size, 8 * 1024 * 1024):
            raise ValueError('Scene metadata exceeds the bounded index limit')
        stream.seek(0)
        metadata = stream.read(base)
    names, end = [], 0
    for index in range(count):
        offset, length = struct.unpack_from('<ii', metadata, h[13] + 8 * index)
        start = h[15] + 2 * offset
        if offset < 0 or length <= 0 or start + 2 * length > base:
            raise ValueError('Invalid scene name extent')
        names.append(metadata[start:start + 2 * length].decode('utf-16-le') + '.dat')
        offset, length = struct.unpack_from('<ii', metadata, h[17] + 8 * index)
        if offset != end or length <= 0 or base + offset + length > size:
            raise ValueError('Invalid scene payload extent')
        end += length
    return safe_members(names)


def safe_members(names, allow_duplicates=False):
    result, seen = [], set()
    for name in names:
        parts = name.replace('\\', '/').split('/')
        if any(not part or part in ('.', '..') or part[-1] in ' .' or
               any(ord(c) < 32 or c in ':*?"<>|' for c in part) or
               pathlib.PureWindowsPath(part).is_reserved() for part in parts):
            raise ValueError('Archive contains an unsafe member name')
        name = '/'.join(parts)
        if name.casefold() in seen and not allow_duplicates:
            raise ValueError('Archive contains duplicate member names')
        seen.add(name.casefold())
        result.append(name)
    return result


def selected(names, pattern):
    return [name for name in names if not pattern or
            fnmatch.fnmatchcase(name.replace('\\', '/').casefold(), pattern.casefold()) or
            fnmatch.fnmatchcase(name.replace('\\', '/').rsplit('/', 1)[-1].casefold(), pattern.casefold())]


def mod_members(data):
    try:
        text = data.decode('utf-8')
    except UnicodeDecodeError:
        text = data.decode('cp932')
    names = re.findall(r'^\s*\d+\s+\[[0-9a-fA-F]+\]\s+([^\r\n]+?)\s*$', text, re.M)
    safe_members(names, allow_duplicates=True)
    # Mod compares exact native member paths; keep its separators for CLI argv.
    return names


def argv_batches(names):
    batch, units = [], 0
    for name in names:
        cost = len(name.encode('utf-16-le')) // 2 + 3
        if units + cost > 12000 and batch:
            yield batch
            batch, units = [], 0
        batch.append(name)
        units += cost
    if batch:
        yield batch
