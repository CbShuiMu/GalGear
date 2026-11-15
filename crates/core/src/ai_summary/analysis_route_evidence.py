"""将玩家可见姓名和选项绑定到本轮实际读取结果，而非模型的转写或概述。"""
import json
from pathlib import Path
import re
import runpy

SOURCES=runpy.run_path(str(Path(__file__).with_name('analysis_sources.py')))


def without_code_comments(text):
    """仅排除明确的 C 风格注释，保留引号字符串中的斜线；不尝试解释未知脚本。"""
    pattern=r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\'|/\*[\s\S]*?\*/|//[^\r\n]*'
    return re.sub(pattern,lambda match:'' if match[0].startswith(('//','/*')) else match[0],text)


def source_value(agent, basis, *, allow_save=False):
    source, pointer = basis.get('source'), basis.get('pointer')
    if not isinstance(source,str) or not isinstance(pointer,str):
        raise ValueError('Text evidence requires source (registered reader source/output or UI receipt) and JSON pointer')
    normalize=SOURCES['normalized_path']
    images=[item for item in getattr(agent,'image_receipts',set()) if normalize(item)==normalize(source)]
    image=bool(images)
    if image:
        document=json.loads(agent.host.path('work',images[0]).read_text('utf-8'))
    else:
        known=SOURCES['reader_records'](agent)
        try:
            roots={record['root'] for record in known.values() if record['root'] in ('game','work') or allow_save and record['root'].startswith('save')}
            output=SOURCES['resolve_reader'](agent,known,source,roots)
            document=json.loads((agent.work/output).read_text('utf-8'))
        except ValueError as error:
            # read_text 已经是本轮原始证据；无需为了同一字符串额外跑一次读取器。
            matches=[]
            for ref,event in getattr(agent,'observations',{}).items():
                args=event.get('arguments',{})
                if event.get('tool')!='read_text' or event.get('error') or args.get('root') not in ('game','work'):continue
                aliases={ref,args.get('path',''),args.get('root','')+'/'+args.get('path','')}
                if normalize(source) in {normalize(item) for item in aliases}:matches.append(event['result'])
            if not matches:raise ValueError(str(error)+' Text source cannot be a model query JSON: '+source) from error
            document=matches[-1]
    if isinstance(document,dict) and isinstance(document.get('text'),str) and pointer and not pointer.startswith('/'):
        pointer='/text'
        basis['pointer']=pointer
    value = agent.api['json_pointer_value'](document,pointer)
    return value, image


def guide_sources(agent,refs):
    """由模型已明确归因为攻略的观察建立来源集合，不根据游戏名或扩展名猜用途。"""
    if not refs:return set()
    normalize=SOURCES['normalized_path']
    aliases=set();inputs=set()
    for ref in refs:
        event=getattr(agent,'observations',{}).get(ref,{})
        if event.get('error'):continue
        args=event.get('arguments',{})
        aliases.add(normalize(ref))
        if event.get('tool')=='read_text' and args.get('root') in ('game','work'):
            path=args.get('path','')
            inputs.add(normalize(agent.host.path(args['root'],path)))
            aliases.update((normalize(path),normalize(args['root']+'/'+path)))
        for record in event.get('result',{}).get('records',[]):
            if record.get('input'):inputs.add(normalize(record['input']))
    if not inputs:return aliases
    for output,record in SOURCES['reader_records'](agent).items():
        if normalize(record['input']) in inputs:
            aliases.update(SOURCES['source_aliases'](agent,record,output))
    return aliases|inputs


def find_name_basis(agent,name):
    """为已存在的原文姓名补齐指针，不从查询结果或内部拼音推断汉字。"""
    if not isinstance(name,str) or not 1<=len(name)<=40:return None
    compact=re.sub(r'\s+','',name)
    if not compact:return None
    pattern=re.compile(r'[ \t\u3000]*'.join(map(re.escape,compact)))
    def visit(value,pointer=''):
        if isinstance(value,str):
            match=pattern.search(value)
            if match:return pointer,match.group()
        elif isinstance(value,(dict,list)):
            for key,item in (value.items() if isinstance(value,dict) else enumerate(value)):
                found=visit(item,pointer+'/'+str(key).replace('~','~0').replace('/','~1'))
                if found:return found
        return None
    for ref,event in getattr(agent,'observations',{}).items():
        if (event.get('tool')=='read_text' and not event.get('error')
                and event.get('arguments',{}).get('root') in ('game','work')):
            found=visit(event.get('result',{}))
            if found:return dict(source=ref,pointer=found[0],name=found[1],source_binding='located_in_original_read')
    for output,record in SOURCES['reader_records'](agent).items():
        if record['root'] not in ('game','work') and not record['root'].startswith('save'):continue
        document=json.loads(agent.host.path('work',output).read_text('utf-8'))
        found=visit(document)
        if found:return dict(source=output,pointer=found[0],name=found[1],source_binding='located_in_original_read')
    return None


def bind_graph(agent, graph, readable_options, guide_refs=()):
    """原位规范化已验证的引用；重复 finish 提交保持幂等。"""
    characters = graph.get('characters')
    if not isinstance(characters,list) or len(characters)>32:
        raise ValueError('graph.characters is required (empty only when no personal names). Each {id,name,source,pointer} must quote an actual speaker/name table, never guess kanji from an internal ID.')
    canonical = {}
    for item in characters:
        name, key = item.get('name'),item.get('id')
        if not isinstance(key,str) or not re.fullmatch(r'[A-Za-z0-9_-]{1,32}',key) or key in canonical:
            raise ValueError('Character IDs must be unique ASCII identifiers')
        try:
            value,_ = source_value(agent,item,allow_save=True)
        except ValueError:
            found=find_name_basis(agent,name)
            if not found:raise
            item.update(found); name=item['name']
            value,_=source_value(agent,item,allow_save=True)
        if isinstance(name,str) and isinstance(value,str) and name not in value:
            compact=re.sub(r'\s+','',name)
            match=re.search(r'[ \t\u3000]*'.join(map(re.escape,compact)),value) if compact else None
            if match:name=item['name']=match.group()
        if not isinstance(name,str) or not 1<=len(name)<=40 or not isinstance(value,str) or name not in value:
            found=find_name_basis(agent,name)
            if not found:raise ValueError('Character name must occur verbatim in the referenced source string: '+key)
            item.update(found); name=item['name']
        canonical[key] = name
    def expand(text):
        def replacement(match):
            key = match[1]
            if key not in canonical: raise ValueError('Unknown character placeholder: '+key)
            return canonical[key]
        return re.sub(r'\{(?:char:)?([A-Za-z0-9_-]+)\}',replacement,text)
    for node in graph['nodes']:
        node['label'] = expand(node['label'])
        if re.search(r'\{[A-Za-z0-9_:-]+\}',node['label']):
            raise ValueError('Unresolved character placeholder; use {char:id} from graph.characters: '+node['label'])
    known = {text for _,option in readable_options for text in option['variants']}
    guides=guide_sources(agent,guide_refs)
    for edge in graph['edges']:
        edge['label'] = expand(edge['label'])
        if 'condition' in edge: edge['condition']=expand(edge['condition'])
        if edge.get('kind')!='choice': continue
        basis = edge.get('text_basis')
        if not basis and edge['label'] in known: continue
        if not isinstance(basis,dict):
            raise ValueError('Clickable choice needs verbatim text_basis={source,pointer,kind:script|ocr|icon,binding:actual UI event/frame-to-target location}. Do not substitute a destination description for original wording. Automatic conditions belong to flow edges.')
        if edge['label'] not in known and SOURCES['normalized_path'](basis.get('source','')) in guides:
            raise ValueError('This source was identified as a walkthrough, which supports route order/navigation but does not prove the actual clickable caption. Read the invoked UI and its referenced button images via read_ui_images; textless portraits use kind=icon with the source-bound character. Keep guide locations in hints/evidence, not the choice label. Do not relabel this source as script: '+str(basis.get('source')))
        value,image = source_value(agent,basis)
        kind = basis.get('kind')
        if kind not in ('script','ocr','icon') or image != (kind in ('ocr','icon')):
            raise ValueError('Text basis kind does not match the registered source')
        if not basis.get('binding'):
            raise ValueError('Choice text needs the actual menu/UI event and destination binding, not image order')
        if kind=='icon':
            if not image or not isinstance(value,dict) or not value.get('sha256') or 'ocr' not in value or value.get('text'):
                raise ValueError('Icon choices must cite a decoded/OCR-inspected frame with no detected text')
            key = basis.get('character_id')
            if key not in canonical: raise ValueError('A character icon needs a source-bound character_id')
            edge['label'] = canonical[key] + '（人物图标）'
        else:
            quote=basis.get('quote')
            if quote is None and kind=='script' and isinstance(value,str) and edge['label'] and edge['label'] in value:
                quote=edge['label']
                basis['quote']=quote
            if quote is not None:
                if kind!='script' or not isinstance(value,str) or not isinstance(quote,str) or not quote or quote not in value:
                    raise ValueError('quote must occur verbatim in the selected original script string; OCR labels use the complete detected text')
                if quote not in without_code_comments(value):
                    raise ValueError('The quoted label occurs only in source comments, not displayed menu text. Follow the actual UI resource and read its button text/image; do not publish internal character/variable descriptions as clickable wording: '+quote)
                value=quote
            if kind=='ocr' and isinstance(value,str) and edge['label']!=value.strip() and isinstance(basis.get('correction_basis'),dict):
                corroboration,other_image=source_value(agent,basis['correction_basis'])
                if not other_image and isinstance(corroboration,str) and edge['label'] and edge['label'] in corroboration:
                    basis['raw_ocr']=value
                    edge['text_status']='ocr_corrected_from_source'
                    value=edge['label']
            if not isinstance(value,str) or edge['label'] != value.strip():
                raise ValueError('Choice must copy the source text exactly. For a larger text field, text_basis.quote may select a verbatim substring with an explicit menu/event binding; do not rewrite or translate it. Image/icon choices require read_ui_images, not a walkthrough destination. source='+str(basis.get('source'))+' pointer='+str(basis.get('pointer'))+' actual='+json.dumps(value,ensure_ascii=False)[:160])
    return canonical
