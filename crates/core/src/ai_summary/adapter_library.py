"""Fixed reusable engine adapters; verification lives outside this library.

An adapter interprets bytes and emits the same evidence as a model-written
parser. Its output never grants a verification receipt or enables preparation.
Only trusted bundled modules may be registered; profiles are JSON data.
"""
import json
import pathlib
import runpy


MAX_INPUT = 32 * 1024 * 1024
MAX_PROFILE = 64 * 1024
MAX_OUTPUT = 64 * 1024 * 1024
MAX_PROGRAM = 128000
_REGISTRY = {
    'entis_cotopha': dict(module='entis_adapter.py', version=1,
        summary='Bounded Entis/CSX tables and versioned Cotopha save state; explicit semantic profile and independent byte proofs required.'),
}
_LOADED = {}


def _json(value, maximum, label):
    try:
        encoded = json.dumps(value, ensure_ascii=False, allow_nan=False)
    except (TypeError, ValueError, RecursionError) as error:
        raise ValueError(label + ' must contain ordinary JSON data') from error
    if len(encoded.encode('utf-8')) > maximum:
        raise ValueError(label + ' exceeds its byte limit')
    return json.loads(encoded)


def capabilities():
    return [dict(id=key, version=record['version'], summary=record['summary'])
            for key, record in _REGISTRY.items()]


def _implementation(adapter):
    if not isinstance(adapter, str) or adapter not in _REGISTRY:
        raise ValueError('Unknown bundled engine adapter; call the engine_adapter_contract tool for registered IDs')
    if adapter not in _LOADED:
        # The model never supplies a module path, import name or command.
        module = pathlib.Path(__file__).with_name(_REGISTRY[adapter]['module'])
        _LOADED[adapter] = runpy.run_path(str(module))
    return _LOADED[adapter]


def describe(adapter):
    value = _implementation(adapter)['describe']()
    return _json(value, MAX_PROFILE, 'Adapter description')


def _invoke(adapter, operation, kind, data, metadata, profile):
    implementation = _implementation(adapter)
    if kind not in ('script', 'save'):
        raise ValueError('Adapter kind must be script or save')
    if not isinstance(data, (bytes, bytearray)) or not 0 < len(data) <= MAX_INPUT:
        raise ValueError('Adapter input must be nonempty bytes of at most 32 MiB')
    profile = _json(profile if profile is not None else {}, MAX_PROFILE, 'Adapter profile')
    metadata = _json(metadata if metadata is not None else {}, MAX_OUTPUT, 'Adapter metadata')
    result = implementation[operation + '_' + kind](bytes(data), metadata, profile)
    return _json(result, MAX_OUTPUT if operation == 'extract' else 1024 * 1024, 'Adapter output')


def inspect(adapter, kind, data, metadata=None, profile=None):
    return _invoke(adapter, 'inspect', kind, data, metadata, profile)


def extract(adapter, kind, data, metadata=None, profile=None):
    return _invoke(adapter, 'extract', kind, data, metadata, profile)


def program(adapter, kind, profile=None):
    """Snapshot the adapter into an ordinary independently tested parser.

    Final parsers do not import this library. Their existing artifact SHA binds
    the actual implementation; adding another adapter cannot change a cached
    parser's behavior. The only external byte capability is galgear_codecs.
    """
    implementation = _implementation(adapter)
    if kind not in ('script', 'save'):
        raise ValueError('Adapter kind must be script or save')
    profile = _json(profile if profile is not None else {}, MAX_PROFILE, 'Adapter profile')
    source = implementation['program'](kind, profile)
    if not isinstance(source, str) or not source.strip() or len(source.encode('utf-8')) > MAX_PROGRAM:
        raise ValueError('Adapter program must be nonempty source of at most 128000 bytes')
    return source
