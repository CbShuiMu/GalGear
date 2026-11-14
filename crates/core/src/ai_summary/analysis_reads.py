"""批量读取已有格式实现，保留原始结构及输入身份，不授予运行时验证资格。"""
import hashlib
import base64
import json
from pathlib import Path
import runpy


BASE = Path(__file__).parent
READERS = runpy.run_path(str(BASE / 'agent_skill_readers.py'))
TYPED = runpy.run_path(str(BASE / 'typed_evidence.py'))
MAX_BYTES = 32 * 1024 * 1024


def query_worker(config, api):
    """直接在隔离进程加载查询数据，避免大 JSON 经父子进程多次复制。"""
    program=config['program']
    # A one-argument query avoids confusing a binary parser's data parameter
    # with the JSON documents dictionary. Retain the old entry for checkpoints.
    if 'def analyze(' in program and 'def extract(' not in program:
        program+='\n\ndef extract(data, metadata):\n    return analyze(metadata["documents"])\n'
    api['validate_program'](program)
    documents, total = {}, 0
    for name, path in config['items']:
        path = Path(path)
        total += path.stat().st_size
        if total > 96 * 1024 * 1024:
            raise ValueError('Query input exceeds 96 MiB; select fewer files')
        value = json.loads(path.read_text('utf-8'))
        if config.get('pointer'):
            value = api['json_pointer_value'](value, config['pointer'])
        if config.get('fields'):
            if not isinstance(value, dict):
                raise ValueError('fields requires an object after pointer selection')
            value = {key:value[key] for key in config['fields'] if key in value}
        documents[name] = value
    return api['parser_worker'](dict(program=program,data='',metadata=dict(documents=documents)))


def reader_options(data, reader, options):
    """以实际容器规范化参数，使 auto 与等价的显式读取共享本轮结果。"""
    options = dict(options or {})
    reader = (READERS['script_reader'](data[:4096]) or READERS['save_reader'](data)
              or ('structured' if data[:4] in (b'PSB\0', b'mdf\0') or READERS['text_container'](data)
                  else 'text')) if reader == 'auto' else reader
    if reader == 'structured' and not options.get('document'):
        document = READERS['text_container'](data)
        if document is None:
            document = dict(syntax='psb', view=[])
            if data[:4] == b'mdf\0':
                document['frame'] = dict(codec='mdf')
        options['document'] = document
    if reader == 'structured':
        options['document'] = dict(options['document'])
        options['document'].setdefault('view', [])
    return reader, options


def data_shape(value):
    """展示数据的真实入口，避免把 document 解码配置误认作解析结果。"""
    if isinstance(value, dict):
        return {key: (dict(keys=list(child)[:160]) if isinstance(child, dict)
                      else dict(length=len(child), first_keys=list(child[0])[:80] if child and isinstance(child[0],dict) else None) if isinstance(child, list)
                      else child) for key, child in list(value.items())[:24]}
    return dict(length=len(value)) if isinstance(value, list) else value


def structured_choices(value):
    """投影结构化脚本的 selects 表；保留语言原文与坐标，不解释条件或推断目标。"""
    choices=[]
    def visit(item,pointer):
        if isinstance(item,dict):
            for key,child in item.items():
                location=pointer+'/'+str(key).replace('~','~0').replace('/','~1')
                if key=='selects' and isinstance(child,list):
                    for index,option in enumerate(child):
                        if not isinstance(option,dict):continue
                        row={k:option[k] for k in ('text','language','disabled','eval','exp','target','storage','selidx') if k in option}
                        row.update(pointer=location,option_pointer=location+'/'+str(index))
                        choices.append(row)
                else:visit(child,location)
        elif isinstance(item,list):
            for index,child in enumerate(item):visit(child,pointer+'/'+str(index))
    visit(value,'/value')
    return choices


def known_options(choices):
    """仅登记有显式目标的可用项的原文候选；语言优先级仍由模型依据安装信息选择。"""
    result=[]
    for choice in choices:
        if not choice.get('target') or str(choice.get('disabled','')).lower() in ('true','1'):continue
        texts=[]
        def collect(item):
            if isinstance(item,dict):
                text=item.get('text')
                if isinstance(text,str) and text and text not in texts:texts.append(text)
                for key,child in item.items():
                    if key=='language':collect(child)
            elif isinstance(item,list):
                for child in item:collect(child)
        collect(choice)
        if texts:result.append(dict(pointer=choice.get('option_pointer',choice.get('pointer','')),variants=texts))
    return result


def structured_dialogue(value):
    """投影 PSB 场景文本表，保留语言列、原生行号与指针，不推定执行顺序或语言优先级。"""
    if not isinstance(value,dict) or not isinstance(value.get('scenes'),list):
        return []
    scenes=[]
    for scene_index, scene in enumerate(value['scenes']):
        if not isinstance(scene,dict) or not isinstance(scene.get('texts'),list):
            continue
        rows=[]
        for index,row in enumerate(scene['texts']):
            if not isinstance(row,list) or len(row)<3 or not isinstance(row[2],list):
                continue
            variants=[]
            pointer='/value/scenes/'+str(scene_index)+'/texts/'+str(index)
            for language_index, variant in enumerate(row[2]):
                if (isinstance(variant,list) and len(variant)>=2 and isinstance(variant[1],str)
                        and (variant[0] is None or isinstance(variant[0],str))):
                    variants.append(dict(language_index=language_index,speaker=variant[0],text=variant[1],
                        text_pointer=pointer+'/2/'+str(language_index)+'/1'))
            if variants:
                rows.append(dict(record_index=index,pointer=pointer,variants=variants))
        scenes.append(dict(scene_index=scene_index,label=scene.get('label'),records=rows))
    return scenes


def anchor_hints(value):
    """展示显式字幕候选的真实指针；不据字段名授予当前节点资格。"""
    hints=[]
    def visit(item,pointer):
        if not isinstance(item,dict) or len(hints)>=32:
            return
        for key,child in item.items():
            path=pointer+'/'+str(key).replace('~','~0').replace('/','~1')
            if (str(key).casefold() in ('lasttext','caption','currenttext','save_msg')
                    and isinstance(child,str) and 4<=len(child.strip())<=4096):
                hints.append(dict(pointer=path,value=child,level='candidate'))
            elif isinstance(child,dict):
                visit(child,path)
    visit(value,'/value')
    return hints[:32]


def read_one(skill, path, reader='auto', options=None, api=None):
    data = path.read_bytes()
    if not 0 < len(data) <= MAX_BYTES:
        raise ValueError('Input must be 1..32 MiB')
    reader, options = reader_options(data, reader, options)
    if reader in READERS['READERS']:
        result = READERS['worker'](dict(skill=str(skill), input=str(path), reader=reader, options=options or {}))
    elif reader == 'structured':
        document = options['document']
        parsed = TYPED['parse_document'](data, document)
        # Plain data is a projection of the complete parsed tree; raw input stays
        # immutable and can be inspected with typed spans when that is needed.
        result = dict(value=TYPED['plain'](parsed['root']), document=document,
                      limitation='Typed fields establish structure, not execution or current-node semantics.')
        result['choice_texts']=structured_choices(result['value'])
        result['scene_dialogue']=structured_dialogue(result['value'])
        result['anchor_hints']=anchor_hints(result['value'])
    elif reader == 'custom':
        native = READERS['script_reader'](data[:4096]) or READERS['save_reader'](data)
        if native or data[:4] in (b'PSB\0', b'mdf\0'):
            raise ValueError('Use the installed '+(native or 'structured')+' reader for this known format. A runtime/import failure needs host repair, not a replacement model decoder.')
        if api is None or not isinstance(options.get('source'), str):
            raise ValueError('custom requires options.source: def extract(data, metadata) returning JSON')
        # 模型解码仍在限时的只读工作进程内，来源身份由宿主补充，不能由程序授予验证等级。
        value = api['parser_worker'](dict(program=options['source'], data=base64.b64encode(data).decode(),
            metadata=dict(source=path.name, size=len(data))))
        result = dict(value=value, decoder_sha256=hashlib.sha256(options['source'].encode()).hexdigest(),
            limitation='Model-authored decoding is candidate evidence, not validated engine state. Preserve native IDs and record order.')
    elif reader == 'text':
        encoding = (options or {}).get('encoding', 'utf-8-sig')
        result = dict(text=data.decode(encoding), encoding=encoding)
    else:
        raise ValueError('Unknown reader')
    result.update(reader=reader, source_sha256=hashlib.sha256(data).hexdigest(),
                  analysis_only=True, verified_current_node=False)
    if path.read_bytes() != data:
        raise ValueError('Input changed during reading')
    return result


def worker(config, api=None):
    """One disposable process reads a bounded batch without per-file startup."""
    skill = Path(config['skill']).resolve()
    output = Path(config['output']).resolve()
    output.mkdir(parents=True, exist_ok=True)
    items = config['items']
    if not isinstance(items, list) or not 1 <= len(items) <= 512:
        raise ValueError('Batch must contain 1..512 explicit files')
    records, total = [], 0
    cache = config.get('cache', {})
    for item in items:
        path = Path(item['input']).resolve()
        record = dict(input=str(path), name=item['name'])
        try:
            if not path.is_file() or not 0 < path.stat().st_size <= MAX_BYTES:
                raise ValueError('Input must be a nonempty file of at most 32 MiB')
            data = path.read_bytes()
            reader, options = reader_options(data, config.get('reader', 'auto'), config.get('options'))
            read_key = hashlib.sha256(json.dumps([str(path), hashlib.sha256(data).hexdigest(), reader, options], sort_keys=True).encode()).hexdigest()
            previous = cache.get(read_key)
            if previous and Path(previous['output']).is_file():
                if path.read_bytes() != data:
                    raise ValueError('Input changed during reading')
                records.append(dict(previous, name=item['name'], reused=True))
                continue
            result = read_one(skill, path, reader, options, api)
            encoded = json.dumps(result, ensure_ascii=False, allow_nan=False).encode('utf-8')
            total += len(encoded)
            if len(encoded) > MAX_BYTES or total > 256 * 1024 * 1024:
                raise ValueError('Batch decoded data exceeds its byte budget; split the batch')
            target = output / (hashlib.sha256(str(path).encode()).hexdigest()[:24] + '.json')
            target.write_bytes(encoded)
            rows = result.get('dialogue', [])
            projected = result.get('scene_dialogue', [])
            record.update(output=str(target), sha256=result['source_sha256'], reader=result['reader'],
                          dialogue_count=len(rows)+sum(len(scene['records']) for scene in projected), keys=list(result),
                          anchor_hints=result.get('anchor_hints',[]),
                          menu_locations=sorted({r['pointer'] for r in result.get('choice_texts',[]) if r.get('pointer')}),
                          control_fields=[k for k in ('instructions','flow','ast_tree','block_observations','value') if k in result],
                          instruction_count=len(result.get('instructions',[])),
                          choice_count=len(result.get('choice_texts',[])), choice_preview=result.get('choice_texts',[])[:4],
                          known_options=known_options(result.get('choice_texts',[])),
                          preview=rows[:1] if rows else ([dict(scene=projected[0]['label'],records=projected[0]['records'][:1])] if projected else data_shape(result.get('value'))),
                          read_key=read_key, reused=False)
        except ImportError as error:
            raise RuntimeError('Installed reader runtime dependency failed; repair/rebuild the host before analysis: '+str(error)) from error
        except Exception as error:
            record['error'] = str(error)[:1200]
        records.append(record)
    return dict(records=records, succeeded=sum('error' not in r for r in records),
                failed=sum('error' in r for r in records), decoded_bytes=total)
